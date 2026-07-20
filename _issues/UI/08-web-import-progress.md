---
title: "Migrate import and processing progress UI"
labels: ["area:ui", "area:web", "type:migration"]
depends_on: ["UI-01", "UI-05"]
size: small
---

# UI-08 · Imports and processing

## UI map

- Import form → Card, Label, Input, Select, Button.
- Uploaded/processed byte state → Progress with native fallback content.
- Completion, invalid state, and processing messages → Alert; action set → Button Group.

## Legacy replacement

Replace raw import controls, action buttons, progress element, and status panels in `import_surface` and `processing_status_shell` with shared components.

## Acceptance criteria

- [x] Import field contracts and submit behavior remain intact.
- [x] Progress retains native `max`/`value` semantics and readable fallback text.
- [x] Failure and completion states use consistent Alert variants.
