---
title: "Migrate the desktop application shell and navigation"
labels: ["area:ui", "area:desktop", "type:migration"]
depends_on: ["UI-01"]
size: medium
---

# UI-12 · Desktop shell

## UI map

- Application section navigation → Navigation Menu with Button actions.
- Shell panels → Card; connection and recording state → Badge; application feedback → Alert.
- Ordinary titles, copy, and data descriptions remain semantic native content.

## Legacy replacement

Replace desktop `app.css`, raw navigation buttons, top-level panels, and connection/status pills in `App` with `UiStyles`, `NavigationMenu`, `Button`, `Card`, `Badge`, and `Alert`.

## Acceptance criteria

- [x] Desktop CSR emits the same theme as web SSR/hydration.
- [x] Navigation and live state signals are preserved through typed component attributes/events.
- [x] The old Trunk CSS asset and stylesheet are removed.
