---
title: "Migrate control-plane handoff, consent, and integration error pages"
labels: ["area:ui", "area:control-plane", "type:migration"]
depends_on: ["UI-00", "UI-01"]
size: medium
---

# UI-17 · Control-plane utility pages

## UI map

- Desktop application handoff → Card, Button Group, Button, Button Link, and status Alert.
- Browser-extension consent → Card, Alert, hidden Input carriers, Button Link, and submit Button.
- Google Drive callback failure → Card and destructive Alert.
- The HTML document shell embeds the same minified `frame-ui` Tailwind stylesheet and exact component class contract as the Leptos applications.

## Legacy replacement

Move HTML ownership out of `frame-application` and replace its hand-written desktop redirect and extension-consent styles, plus the control-plane integration-error styles, with the runtime-neutral `frame-ui` class contract in the control-plane web adapter. These three static pages deliberately do not link Leptos because `workers-rs` and Leptos currently pull incompatible `wasm-streams` ABIs into a final Worker binary; the full web and desktop applications use the Leptos components.

## Acceptance criteria

- [x] All three server-generated utility pages use the exact shared shadcn-inspired class contract with semantic native markup.
- [x] The utility document embeds the exact shared minified Tailwind bundle and has responsive viewport metadata.
- [x] Redirect, form, CSP, no-store, reflected-value escaping, and progressive fallback behavior remain intact.
- [x] Application/domain crates no longer own UI markup.
- [x] The control-plane release Worker links without Leptos or duplicate `wasm-streams` symbols.
