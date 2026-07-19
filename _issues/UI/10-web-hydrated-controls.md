---
title: "Migrate hydrated auth and player control islands"
labels: ["area:ui", "area:web", "area:player", "type:migration"]
depends_on: ["UI-01", "UI-09"]
size: medium
---

# UI-10 · Hydrated controls

## UI map

- Authenticated workspace browser panel → shared Card utility contract; inputs/actions → Input, Label, Button, Button Group.
- Hydration boundary feedback → Alert/status composition.
- Play/pause/seek/mute/fullscreen controls → Button and Button Group.
- Keyboard-help disclosure → Button with native expanded/controls attributes plus semantic help list.

## Legacy replacement

Replace raw hydrated controls and `.player-controls` action styling in `AuthenticatedWorkspacePanel`, `HydrationBoundary`, and `PlayerKeyboardHelp` with `frame-ui`. The auth panel keeps a native `section` with the exact shared Card utility contract because its `Rc`-capturing Leptos children cannot satisfy the shared `Children: Send` boundary.

## Acceptance criteria

- [x] Hydrated event handlers and DOM properties forward through shared controls.
- [x] `.authenticated-browser-panel`, `.player-controls`, `.player-status`, and `.player-keyboard-help` browser hooks remain stable.
- [x] Controls expose their native type and required ARIA state.
