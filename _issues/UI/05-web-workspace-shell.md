---
title: "Migrate the authenticated workspace shell and navigation"
labels: ["area:ui", "area:web", "type:migration"]
depends_on: ["UI-01", "UI-04"]
size: medium
---

# UI-05 · Workspace shell

## UI map

- Account/header navigation and workspace sidebar → Navigation Menu.
- Member role marker → Badge.
- Main workspace/content frame → Card composition with native `main`, headings, and lists.
- Account/session actions → Button or Button Link.

## Legacy replacement

Replace the authenticated shell's raw `nav`, role pill, local action styling, and generic panels with `NavigationMenu`, `Badge`, `Button`, `ButtonLink`, and shared Card styling. Preserve `.workspace-layout` and `.role-badge` as browser-contract hooks.

## Acceptance criteria

- [x] All role-filtered navigation renders through shared navigation primitives.
- [x] Workspace/member/role ownership remains in the web route and is passed as display content only.
- [x] Browser compatibility selectors and accessibility landmarks are preserved.
