---
title: "Migrate workspace settings, forms, and restricted data surfaces"
labels: ["area:ui", "area:web", "type:migration"]
depends_on: ["UI-01", "UI-05"]
size: medium
---

# UI-07 · Web settings and data surfaces

## UI map

- Settings groups → Card; form captions/controls → Label, Input, Select, Textarea as applicable.
- Save/create/revoke actions → Button/Button Link with destructive intent where needed.
- Role restrictions and unavailable settings → Alert.
- Existing semantic lists and `dl` data remain native within shared card composition.

## Legacy replacement

Replace settings panels, raw controls, local buttons, and restriction notices in `settings_index` and `restricted_surface` with `frame-ui` components.

## Acceptance criteria

- [x] Profile, organization, billing/API/data settings exposed by the route use shared components.
- [x] Form names, values, methods, and role restrictions are unchanged.
- [x] Destructive and restricted operations are visually distinct and accessible.
