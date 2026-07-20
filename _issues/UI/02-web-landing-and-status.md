---
title: "Migrate the public landing and status shells to shared UI"
labels: ["area:ui", "area:web", "type:migration"]
depends_on: ["UI-01"]
size: medium
---

# UI-02 · Web landing and status shells

## UI map

- Site header links → Navigation Menu; hero calls to action → Button Link and Button Group.
- Three product capability tiles → Card (`FeatureCard`).
- Runtime notice → Alert; not-found and generic status content → Card/Empty State composition.
- Headings, explanatory copy, and ordinary navigation links remain semantic native content.

## Legacy replacement

Replace `pages::landing`, `pages::not_found`, and `status_shell` local card/action/notice classes with `NavigationMenu`, `ButtonLink`, `ButtonGroup`, `FeatureCard`, `Alert`, and `EmptyState`.

## Acceptance criteria

- [x] Every action and repeated container on these surfaces uses `frame-ui`.
- [x] SSR output remains useful without JavaScript and preserves link destinations and heading order.
- [x] Responsive layout is supplied by the shared Tailwind bundle.
