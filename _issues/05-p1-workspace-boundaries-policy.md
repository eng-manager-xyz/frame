---
title: "Establish Rust workspace boundaries, ownership, and dependency policy"
labels:
  - "phase:p1"
  - "area:rust"
  - "area:architecture"
  - "type:foundation"
  - "risk:medium"
depends_on: [01, 03]
size: epic
---

# 05 · Establish Rust workspace boundaries, ownership, and dependency policy

## Outcome

The initial scaffold becomes a maintainable workspace with runtime-safe crate boundaries and clear ownership.

## Current Cap reference

Frame currently proves domain, port, GStreamer, Worker, and Leptos seams. Cap's large monorepo demonstrates both the value and cost of many narrow crates, vendored dependencies, and platform-specific forks.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#01](./01-p0-migration-charter-parity-slos.md), [#03](./03-p0-runtime-topology.md)

## Scope

Approve crate responsibilities, dependency direction, feature policy, MSRV, edition, error and tracing conventions, unsafe policy, provenance rules, and CODEOWNERS. Add an xtask or equivalent only where it eliminates repeated cross-platform commands.

### Out of scope

Prematurely reproducing all Cap crates or selecting every future dependency is not required.

## Deliverables

- [ ] A crate/runtime map with allowed dependency directions and examples of forbidden edges.
- [ ] Pinned Rust toolchain and lockfile policy plus native/Wasm feature rules.
- [ ] Dependency, license, vulnerability, source-provenance, and unsafe-code review policy.
- [ ] Ownership metadata and contribution templates for migrations and ADRs.
- [ ] Developer commands for format, lint, host tests, Wasm check, media smoke, and local service startup.

## Acceptance criteria

- [ ] No Worker/Wasm crate depends on GStreamer, GLib, a threaded runtime, or native platform capture.
- [ ] Domain and contract crates compile for native and wasm32 targets.
- [ ] Dependency cycles and duplicate responsibility are absent from the documented graph.
- [ ] A dependency/license audit has an owner and actionable exception process.
- [ ] A new contributor can run documented checks from a clean checkout.

## Required test evidence

- Cargo metadata/dependency graph output.
- Clean-checkout build log on native and wasm32.
- Initial dependency and license inventory.

## Risks and open questions

- Too many crates can hide ownership and slow compilation; too few can leak runtime dependencies.
- An overbroad license choice could conflict with adapted upstream code.

## Rollout and rollback

Boundary changes stay internal until contracts are consumed externally. Revert by moving code without changing serialized schemas.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
