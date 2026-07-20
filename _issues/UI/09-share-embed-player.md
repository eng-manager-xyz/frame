---
title: "Migrate share, embed, and public player UI"
labels: ["area:ui", "area:web", "area:player", "type:migration"]
depends_on: ["UI-01"]
size: medium
---

# UI-09 · Share/embed player

## UI map

- Share header/navigation → Navigation Menu; public media surface → Feature Card.
- Video viewport → Aspect Ratio wrapping the native `video` and caption `track` elements.
- Player/share actions → Button Link/Button Group; state and policy messages → Alert/Badge.
- Transcript/captions and metadata retain native headings, lists, and text semantics.

## Legacy replacement

Replace share/embed local navigation, `.video-frame`, feature panel, notices, and action styles in `share`, `embed`, and `public_player_shell` with shared components.

## Acceptance criteria

- [x] Public, unavailable, processing, and embed-policy states remain server-rendered.
- [x] Video dimensions use the shared aspect-ratio primitive without weakening native media/caption controls.
- [x] CSP-sensitive URLs and embed behavior are unchanged.
