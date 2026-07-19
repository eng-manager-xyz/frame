---
title: "Migrate desktop settings, presets, lifecycle, updates, and error dialog"
labels: ["area:ui", "area:desktop", "type:migration"]
depends_on: ["UI-01", "UI-12"]
size: medium
---

# UI-16 · Desktop settings and dialog

## UI map

- Settings/preset groups → Card and Field Group with Label/Input/Select.
- Save/delete/check/update/lifecycle actions → Button Group and intent variants.
- Legacy configuration warning and application status → Alert.
- Blocking error scrim/panel → Dialog Overlay and Dialog Content with labelled heading and close Button.

## Legacy replacement

Replace raw settings controls/action rows, legacy aside notice, lifecycle/update status, and `.modal-backdrop`/`.modal` implementation in desktop `App` with shared components.

## Acceptance criteria

- [x] All setting, preset, lifecycle, and update controls use shared primitives.
- [x] Legacy warnings and failures use accessible Alert semantics.
- [x] Dialog role, modal state, labelled heading, close action, and focusable native control are preserved.
