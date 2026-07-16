---
title: "Rebuild Leptos auth, onboarding, dashboard, library, spaces, folders, settings, import, developer, billing, and admin surfaces"
labels:
  - "phase:p5"
  - "area:leptos"
  - "area:web"
  - "area:accessibility"
  - "type:migration"
  - "risk:medium"
depends_on: [08, 13, 14, 15, 30]
size: epic
---

# 31 · Rebuild Leptos auth, onboarding, dashboard, library, spaces, folders, settings, import, developer, billing, and admin surfaces

## Outcome

Authenticated web workflows move from Next/React to accessible Leptos pages with approved visual, functional, SEO, and performance parity.

## Current Cap reference

Cap's web application includes login/signup/OTP, onboarding, dashboard/library, folders, spaces, organization and user settings, imports, storage, developers, billing, analytics, admin, and many supporting components/actions.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#08](./08-p1-leptos-web-desktop-shells.md), [#13](./13-p2-auth-sessions-identity.md), [#14](./14-p2-organizations-rbac-spaces-folders.md), [#15](./15-p2-video-collaboration-business-data.md), [#30](./30-p5-rust-api-workflow-parity.md)

## Scope

Build routed Leptos surfaces, session bootstrap, forms, validation, optimistic updates where safe, pagination/search/filter, empty/loading/error states, upload/import progress, settings, billing/developer/admin gates, responsive design, keyboard/screen-reader behavior, telemetry consent, and route redirects.

### Out of scope

Public share/player surfaces are issue 32; desktop recorder/editor is issue 33; broad marketing/blog/docs migration requires an explicit charter disposition.

## Deliverables

- [ ] Page-by-page route/component matrix and reusable accessible design-system primitives.
- [ ] Leptos loaders/actions or typed API clients with cache invalidation and error mapping.
- [ ] Authenticated layouts, navigation, focus management, responsive states, and permission-aware controls.
- [ ] Visual/E2E/accessibility fixtures for critical user journeys and role combinations.
- [ ] Legacy URL, query, deep-link, metadata, and redirect compatibility plan.

## Acceptance criteria

- [ ] Every retained authenticated route completes its critical journey under owner/admin/member and denied states.
- [ ] Forms preserve validation, pending, duplicate-submit, retry, unsaved-change, and server-error behavior.
- [ ] WCAG target automated scans plus keyboard/screen-reader manual checks pass for critical flows.
- [ ] SSR/hydration or chosen rendering has no mismatch, secret-in-HTML, flash-of-private-content, or broken no-JS requirement.
- [ ] Bundle, navigation, server render, API, and interaction performance remain inside charter budgets.

## Required test evidence

- Route/journey parity matrix and E2E report.
- Visual diffs at supported breakpoints and themes.
- Accessibility and web-performance reports.

## Risks and open questions

- Migrating breadth before design primitives stabilizes creates duplication.
- Optimistic UI can display unauthorized or stale state if policy/cache boundaries are wrong.

## Rollout and rollback

Release route families behind user/tenant flags with legacy deep-link fallback. Roll back routing without reverting D1/API data.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
