---
title: "Migrate public collaboration and transcript UI"
labels: ["area:ui", "area:web", "type:migration"]
depends_on: ["UI-01", "UI-09"]
size: medium
---

# UI-11 · Public collaboration

## UI map

- Collaboration surface/fallback → Card/Alert composition.
- Transcript and comment entries → native lists/headings inside cards.
- Comment body → Textarea; consent → checkbox Input with Label; post action → Button.
- Loading, success, and failure feedback → Alert/status semantics.

## Legacy replacement

Replace raw textarea, checkbox, label, post button, and bespoke collaboration feedback styling in `PublicCollaborationPanel` with shared components. Preserve `.public-collaboration`, `.collaboration-fallback`, and `.collaboration-status` as browser-contract hooks.

## Acceptance criteria

- [x] Existing async collaboration behavior and consent requirements are preserved.
- [x] Form controls are labelled and keyboard operable.
- [x] Progressive-enhancement fallback remains visible when JavaScript is unavailable.
