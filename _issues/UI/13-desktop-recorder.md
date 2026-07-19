---
title: "Migrate desktop recorder configuration and capture controls"
labels: ["area:ui", "area:desktop", "area:recorder", "type:migration"]
depends_on: ["UI-01", "UI-12"]
size: large
---

# UI-13 · Desktop recorder

## UI map

- Instant/Studio mode, capture target, and display choices → Toggle Group with Button pressed state.
- Permission/device configuration → Card/Card Frame, Field Group, Label, Select, Input.
- Microphone/camera activity → Meter; preparation/capture status → Badge/Alert.
- Start/pause/resume/stop/cancel controls → Button Group with intent variants.

## Legacy replacement

Replace `.button-row`, raw fieldsets/labels/selects/inputs/meters/buttons, `.permission-card`, and recorder status styling in the recorder sections of desktop `App` with shared components.

## Acceptance criteria

- [x] Every recorder mode, target, display, permission, and device control is mapped.
- [x] `aria-pressed`, labels, descriptions, disabled state, values, and IPC event handlers are preserved.
- [x] Signal meters retain native min/max/value semantics.
