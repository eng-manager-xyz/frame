---
title: "Migrate desktop editor, timeline, export, and upload UI"
labels: ["area:ui", "area:desktop", "area:editor", "type:migration"]
depends_on: ["UI-01", "UI-12"]
size: large
---

# UI-15 · Editor, export, and upload

## UI map

- Editor and timeline groups → Card/Field Group; numeric/range/color adjustments → Label and Input.
- Timeline and transport actions → Button/Toggle Group/Button Group.
- Export/upload completion → Progress; errors/results → Alert; file/share actions → Button.

## Legacy replacement

Replace editor/timeline raw controls, local panels/action rows, export/upload progress elements, and result/error styles in desktop `App` with shared components.

## Acceptance criteria

- [x] Every editor adjustment and timeline control is represented by a shared input/action primitive.
- [x] Bounds, steps, values, disabled states, labels, and IPC callbacks are preserved.
- [x] Export/upload progress and failure states are consistent with import/instant-share UI.
