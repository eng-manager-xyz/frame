---
title: "Create the shared shadcn-inspired Leptos primitive crate"
labels: ["area:ui", "area:leptos", "type:component"]
depends_on: ["UI-00"]
size: large
---

# UI-01 · Shared Leptos primitives

## Outcome

`frame-ui` supplies typed, accessible, reusable Leptos components for all styles repeated across web and desktop while keeping product state in the applications.

## Mapping and legacy replacement

- Buttons/action links → shadcn Button → `Button`, `ButtonLink`, variants and sizes; replace raw controls and local `.primary`/`.danger` styling.
- Form controls → shadcn Input, Select, Textarea, Label, Field → `Input`, `Select`, `Textarea`, `Label`, `FieldGroup`; replace raw form elements.
- Containers → shadcn Card/Empty → `Card`, `FeatureCard`, `CardFrame`, `EmptyState`; replace `.panel`, `.feature-card`, `.permission-card`, and empty-result frames.
- Feedback → shadcn Badge/Alert/Progress → `Badge`, `Alert`, `Progress`, `Meter`; replace `.notice`, `.status`, `.state`, and bespoke progress/meter styling.
- Structure → shadcn Navigation Menu, Button Group, Toggle Group, Separator, Aspect Ratio, Dialog → the corresponding `frame-ui` components; replace repeated wrapper markup and modal styling.

## Acceptance criteria

- [x] The crate follows workspace Rust 2024/resolver 3 conventions and exposes cohesive component modules.
- [x] Variants are enums rather than stringly typed product flags, and arbitrary DOM attributes/events forward to the native root.
- [x] Components retain native accessibility contracts and render in SSR, hydration, and CSR feature modes.
- [x] Tests prove semantic output, theme tokens, minification, and stylesheet budget.
- [x] Shared APIs stay independent of application workflow/domain state.
