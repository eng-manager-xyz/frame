---
title: "Package a minimal audited GStreamer runtime and plugin set for macOS, Windows, and Linux"
labels:
  - "phase:p4"
  - "area:gstreamer"
  - "area:desktop"
  - "area:release"
  - "type:migration"
  - "risk:high"
depends_on: [03, 05, 09, 10]
size: epic
---

# 22 · Package a minimal audited GStreamer runtime and plugin set for macOS, Windows, and Linux

## Outcome

Supported Frame desktop and media-worker builds discover a reproducible, legally reviewed GStreamer runtime without relying on an accidental developer installation.

## Current Cap reference

The scaffold probes a host GStreamer installation and uses videotestsrc, videoconvert, vp8enc, webmmux, and filesink. Production needs capture bridges, hardware/software codecs, muxers, playback, analysis, and cross-platform distribution. Codec and plugin licenses differ.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#03](./03-p0-runtime-topology.md), [#05](./05-p1-workspace-boundaries-policy.md), [#09](./09-p1-ci-quality-gates.md), [#10](./10-p1-local-development-stack.md)

## Scope

Define minimum GStreamer version, plugin/factory manifest, packaging source, search paths, updates, signing/notarization, architecture support, optional capabilities, hardware codec discovery, software fallback, SBOM, licenses, CVE response, and installer sizes.

### Out of scope

Pipeline behavior is issue 23 onward. Adding every plugin bundle is explicitly not a goal.

## Deliverables

- [ ] A capability-to-plugin/factory manifest with required, optional, prohibited, and platform-specific entries.
- [ ] Reproducible runtime bundles/installers or an approved system-dependency policy for each target.
- [ ] Startup doctor with actionable missing/incompatible plugin diagnostics.
- [ ] License notices, source-offer/redistribution obligations, patent/codec decision, SBOM, and vulnerability process.
- [ ] Signed build and update path tested on clean machines for all supported architectures.

## Acceptance criteria

- [ ] A clean supported machine runs the synthetic A/V smoke without Homebrew, MSYS, or developer-only paths unless system dependencies are the approved product model.
- [ ] Missing optional hardware support falls back to an approved software path; missing required components fail before recording.
- [ ] Plugin search paths cannot load untrusted user-writable binaries ahead of the signed bundle.
- [ ] Runtime, application, and plugin versions are included in privacy-safe diagnostics.
- [ ] Packaging passes code signing/notarization and license/security review on every supported target.

## Required test evidence

- Clean VM install/run/uninstall logs for each OS/architecture.
- Factory manifest diff and SBOM.
- Hardware encoder availability/fallback matrix.

## Risks and open questions

- GStreamer redistribution and H.264/AAC patent/licensing choices can block release.
- Plugins can greatly increase installer size and attack surface.

## Rollout and rollback

Distribute in internal/nightly channels first. Preserve the legacy media backend and bundle version pin for rollback.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
