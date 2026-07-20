# Bespoke shadcn/Leptos UI migration

This directory is the implementation record for Frame's complete user-interface census. Each issue groups one cohesive product surface, maps every rendered control or container to a shadcn design-system analogue, and names the legacy use that the shared Leptos component replaces. The implementation is bespoke Rust/Leptos; shadcn is the visual and accessibility reference, not a JavaScript runtime dependency.

All shared primitives live in `crates/ui`. Tailwind CSS v4 scans the shared crate, both Leptos applications, and the three control-plane utility pages, then emits one minified, bounded stylesheet that every surface embeds from the same Rust constant. Product views retain workflow state, authorization, copy, and semantic lists; the UI crate owns reusable visual variants, native-control defaults, and the exact runtime-neutral class contract needed by the Worker adapter.

## Component map

| Existing UI piece | shadcn analogue | Frame component or retained semantic element | Migration issue |
|---|---|---|---|
| Primary, secondary, outline, ghost, destructive, and compact actions | Button | `Button`, `ButtonVariant`, `ButtonSize` | [UI-01](./01-shared-primitives.md) |
| Links styled as actions | Button `asChild` | `ButtonLink` | [UI-01](./01-shared-primitives.md) |
| Related actions and pressed-value choices | Button Group / Toggle Group | `ButtonGroup`, `ToggleGroup` | [UI-01](./01-shared-primitives.md) |
| Text, email, password, search, number, range, color, checkbox controls | Input | `Input` with native `type` | [UI-01](./01-shared-primitives.md) |
| Native option selection | Select | `Select` | [UI-01](./01-shared-primitives.md) |
| Multiline comments | Textarea | `Textarea` | [UI-01](./01-shared-primitives.md) |
| Form labels and grouped controls | Label / Field | `Label`, `FieldGroup` | [UI-01](./01-shared-primitives.md) |
| Panels, feature tiles, framed content, empty results | Card / Empty | `Card`, `FeatureCard`, `CardFrame`, `EmptyState` | [UI-01](./01-shared-primitives.md) |
| Role, state, connection, and recording markers | Badge | `Badge`, `BadgeVariant` | [UI-01](./01-shared-primitives.md) |
| Notices, restrictions, errors, legacy warnings, status | Alert | `Alert`, `AlertVariant` | [UI-01](./01-shared-primitives.md) |
| Header, workspace, player, and desktop navigation | Navigation Menu | `NavigationMenu` | [UI-01](./01-shared-primitives.md) |
| Upload, import, export, and processing completion | Progress | `Progress` | [UI-01](./01-shared-primitives.md) |
| Microphone/camera signal levels | Meter | `Meter` | [UI-01](./01-shared-primitives.md) |
| Error modal and scrim | Dialog | `DialogOverlay`, `DialogContent` | [UI-01](./01-shared-primitives.md) |
| Video viewport | Aspect Ratio | `AspectRatio` around native `video` | [UI-09](./09-share-embed-player.md) |
| Dividers | Separator | `Separator` | [UI-01](./01-shared-primitives.md) |
| Data rows, description lists, ordered/unordered lists, headings, paragraphs, video, and captions | shadcn composition with native semantics | Native element composed inside shared primitives | Surface issue below |

## Surface census and issue index

| ID | Source surface | UI pieces covered |
|---|---|---|
| UI-00 | Build/theme foundation | Tailwind CLI, tokens, dark/system themes, reduced motion, forced colors, minification, bundle budget |
| UI-01 | Shared primitive crate | Every reusable control/container listed in the component map |
| UI-02 | `pages::landing`, not-found/status shells | Header navigation, hero actions, feature cards, notices, empty/error state |
| UI-03 | `pages::{login,recovery,signup,verify}` | Auth cards, labels, inputs, submit actions, feedback alerts, cross-flow links |
| UI-04 | `pages::{authenticated,authenticated_at,private_status_shell}` | loading, unauthorized, forbidden, unavailable, and not-found boundaries |
| UI-05 | Authenticated workspace shell | account header, workspace sidebar navigation, role badge, main content frame |
| UI-06 | `recording_library`, collection/detail surfaces | search, state filter, recording cards/links, status badges, empty state, restricted detail |
| UI-07 | `settings_index`, restricted settings/data screens | setting cards, profile/API/billing forms, selects, inputs, restrictions, native data lists |
| UI-08 | `import_surface`, processing status | import fields/actions, progress, completion/error alerts |
| UI-09 | `share`, `embed`, `public_player_shell` | share header, media card, aspect-ratio video, captions/transcript, embed/player state |
| UI-10 | `AuthenticatedWorkspacePanel`, `HydrationBoundary`, `PlayerKeyboardHelp` | hydrated auth actions, player controls, grouped actions, keyboard-help disclosure |
| UI-11 | `PublicCollaborationPanel` | collaboration fallback, transcript, comments, textarea, consent input, post action/status |
| UI-12 | desktop `App` shell | application navigation, connectivity/recording badges, top-level status and panels |
| UI-13 | desktop recorder configuration | mode/target/display toggles, permission/device cards, labels, selects, meters, record controls |
| UI-14 | desktop instant sharing and recovery | progress/error feedback, copy/open actions, recovery cards and decisions |
| UI-15 | desktop editor, timeline, export, upload | adjustment inputs, timeline controls, progress, action groups, results |
| UI-16 | desktop settings, presets, lifecycle, update, modal error | field groups, legacy alert, save/update actions, status alert, dialog |
| UI-17 | control-plane desktop handoff, extension consent, integration error | cards, alerts, hidden inputs, action groups, links, buttons, shared document theme |

## Migration invariants

- Application views do not render raw `button`, `input`, `select`, `textarea`, `progress`, `meter`, `label`, `nav`, or `fieldset` elements; those native elements are emitted by `frame-ui`.
- Native document semantics remain native where a design-system wrapper would reduce accessibility: headings, paragraphs, lists, `dl`, tables, links that are not actions, `video`, tracks, and legends.
- Each surface issue below explicitly replaces its prior local class-based implementation and is complete only when its application call sites use `frame-ui`.
- Browser-test hooks such as `.workspace-layout`, `.role-badge`, `.state`, `.player-controls`, and `.public-collaboration` are compatibility selectors, not component styling ownership.
- The committed generated stylesheet must exactly match the pinned Tailwind input and stay below the declared size budget.
- Server-generated control-plane utility pages use the exact `frame-ui` class contract and stylesheet with semantic native markup. They remain static because linking Leptos beside `workers-rs` introduces incompatible `wasm-streams` ABIs; application/domain crates contain no UI markup.
