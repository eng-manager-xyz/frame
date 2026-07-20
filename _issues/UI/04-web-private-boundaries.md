---
title: "Migrate authenticated route boundaries and private status states"
labels: ["area:ui", "area:web", "area:auth", "type:migration"]
depends_on: ["UI-01"]
size: small
---

# UI-04 · Private route boundaries

## UI map

- Loading/unavailable/unauthorized/forbidden/not-found boundaries → Card or Empty State.
- Authentication and authorization messages → Alert where the state requires emphasis.
- Recovery/navigation action → Button Link.

## Legacy replacement

Replace local private-shell panel and action markup in `authenticated`, `authenticated_at`, and `private_status_shell` with shared Card/Empty State/Alert/Button Link composition.

## Acceptance criteria

- [x] Every private route boundary has a consistent shared visual treatment.
- [x] Authorization remains server-derived and is not moved into the component crate.
- [x] Status text and recovery destinations are unchanged.
