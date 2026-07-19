---
title: "Migrate recording library, collection, and detail UI"
labels: ["area:ui", "area:web", "type:migration"]
depends_on: ["UI-01", "UI-05"]
size: medium
---

# UI-06 · Recording library and collections

## UI map

- Query field → Input; state filter → Select; captions → Label.
- Recording/list tiles → Card; recording states → Badge; no matches → Empty State.
- Open/details actions → Button Link; restricted detail → Alert/Card.
- Result lists, metadata `dl`, timestamps, and headings retain native semantics inside cards.

## Legacy replacement

Replace raw controls, state pills, result panels, empty-result panel, and button-like links in `recording_library`, `collection_surface`, and `detail_surface` with shared components.

## Acceptance criteria

- [x] Search/filter server behavior and query values are preserved.
- [x] Every recording state uses a typed Badge variant while `.state` remains a test hook.
- [x] Empty and restricted outcomes use shared Empty State/Alert treatment.
