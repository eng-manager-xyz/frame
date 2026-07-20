---
title: "Migrate desktop instant sharing and recovery UI"
labels: ["area:ui", "area:desktop", "type:migration"]
depends_on: ["UI-01", "UI-12"]
size: medium
---

# UI-14 · Instant sharing and recovery

## UI map

- Instant share state → Card with Progress and Alert/Badge feedback.
- Copy/open/retry/cancel actions → Button Group and Button variants.
- Recoverable recordings → Card list; recovery decisions → Button Group; no recoveries → Empty State.

## Legacy replacement

Replace instant-sharing panels, raw progress/buttons, error styling, recovery panels, and action rows in desktop `App` with shared components.

## Acceptance criteria

- [x] Upload/share and recovery states retain their operation identifiers and backend-driven truth.
- [x] Progress exposes native values and failures use destructive Alert intent.
- [x] All recovery actions remain keyboard accessible and visually grouped.
