---
title: "Scaffold Leptos SSR/hydration web app and Tauri desktop shell"
labels:
  - "phase:p1"
  - "area:leptos"
  - "area:desktop"
  - "type:foundation"
  - "risk:medium"
depends_on: [03, 05, 06]
size: epic
---

# 08 · Scaffold Leptos SSR/hydration web app and Tauri desktop shell

## Outcome

Frame has production-shaped Leptos shells for web and desktop with shared components and distinct HTTP/Tauri transports.

## Current Cap reference

The scaffold contains a static Leptos SSR page served by Axum. Cap uses Next/React for web and SolidStart inside Tauri for desktop. Public pages need metadata while desktop capture needs native commands and permissions.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#03](./03-p0-runtime-topology.md), [#05](./05-p1-workspace-boundaries-policy.md), [#06](./06-p1-shared-domain-api-contracts.md)

## Scope

Choose and prove web SSR/hydration or Worker-compatible rendering, Leptos CSR inside Tauri, shared design tokens/components, typed API and IPC clients, routing, metadata, error boundaries, loading states, and CSP.

### Out of scope

Full dashboard, player, recorder, editor, billing, and admin parity belongs to issues 31–33.

## Deliverables

- [ ] An ADR for Leptos deployment targets and rendering strategy.
- [ ] A hydrating web shell with router, metadata, session bootstrap, and typed HTTP client.
- [ ] A minimal Tauri 2 shell using Leptos CSR and typed IPC without exposing broad global APIs.
- [ ] Shared accessible components, design tokens, responsive layout, and error/loading primitives.
- [ ] Build, development, and smoke-test commands for web and desktop.

## Acceptance criteria

- [ ] Web HTML contains useful content and metadata before JavaScript and hydrates without console errors.
- [ ] Desktop invokes one allowlisted typed Rust command and rejects an unapproved command by capability policy.
- [ ] Shared components compile for their intended targets without pulling server/native code into browser Wasm.
- [ ] Keyboard focus, reduced motion, contrast, and screen-reader landmarks pass the initial accessibility audit.
- [ ] CSP and Tauri capabilities prohibit arbitrary remote script and filesystem access.

## Required test evidence

- Web SSR/hydration and desktop smoke artifacts.
- Accessibility scan and keyboard walkthrough.
- Bundle/dependency reports for each target.

## Risks and open questions

- Experimental Workers-side SSR or incompatible Tokio assumptions can block deployment.
- Sharing too much UI code can blur web and desktop security boundaries.

## Rollout and rollback

Ship shells only on non-production routes/build channels. Roll back to the static scaffold or legacy UI without changing API contracts.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
