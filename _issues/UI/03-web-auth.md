---
title: "Migrate login, recovery, signup, and verification UI"
labels: ["area:ui", "area:web", "area:auth", "type:migration"]
depends_on: ["UI-01"]
size: medium
---

# UI-03 · Web authentication

## UI map

- Auth panel → Card; field captions → Label; email/password/token fields → Input.
- Submit/retry actions → Button; links between auth flows remain native links.
- Validation, recovery, and verification feedback → Alert with destructive/default intent.

## Legacy replacement

Replace raw form controls and local panel/button/notice styles in `pages::{login,recovery,signup,verify}` with `Card`, `Label`, `Input`, `Button`, and `Alert`.

## Acceptance criteria

- [x] All four auth flows use the same shared controls and feedback variants.
- [x] Names, types, autocomplete, required state, labels, actions, and server-side form behavior are preserved.
- [x] Error and success feedback remains available in no-JavaScript SSR.
