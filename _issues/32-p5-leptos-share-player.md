---
title: "Rebuild Leptos share/embed player, privacy, comments, transcripts, and analytics"
labels:
  - "phase:p5"
  - "area:leptos"
  - "area:player"
  - "area:security"
  - "type:migration"
  - "risk:high"
depends_on: [08, 15, 19, 21, 30]
size: epic
---

# 32 · Rebuild Leptos share/embed player, privacy, comments, transcripts, and analytics

## Outcome

Public, private, embedded, and custom-domain playback is fast, accessible, correctly authorized, and compatible with existing links.

## Current Cap reference

Cap serves share and embed routes with playback, metadata, thumbnails, privacy/password/domain behavior, comments, reactions/views, transcripts/captions, analytics, editing links, custom branding, and SEO/OpenGraph output.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#08](./08-p1-leptos-web-desktop-shells.md), [#15](./15-p2-video-collaboration-business-data.md), [#19](./19-p3-multipart-upload-download.md), [#21](./21-p3-storage-security-lifecycle.md), [#30](./30-p5-rust-api-workflow-parity.md)

## Scope

Implement server-rendered metadata, share authorization, passwords if retained, signed/range playback, player controls, keyboard/screen-reader support, captions/transcript, comments/replies/reactions, analytics consent, embed messaging, custom domains/branding, unavailable/deleted states, and cache/privacy behavior.

### Out of scope

Authenticated dashboard/editor surfaces are issues 31/33; media generation is issue 28.

## Deliverables

- [ ] Share/embed contract and privacy-state matrix covering anonymous, authenticated, tenant, password, deleted, processing, failed, and unavailable cases.
- [ ] Accessible player with range/seek, captions, speed, fullscreen/PiP where approved, error recovery, and reduced-motion behavior.
- [ ] Dynamic metadata/OpenGraph/canonical/noindex policy that never leaks private titles/thumbnails.
- [ ] Comment/transcript/analytics flows with moderation, rate limits, consent, and tenant policy.
- [ ] Legacy link, embed API/postMessage, custom-domain, and cache compatibility tests.

## Acceptance criteria

- [ ] Existing public links and embeds resolve or redirect correctly; private/deleted/forbidden content never leaks metadata, thumbnails, object URLs, or existence beyond policy.
- [ ] Seeking and playback work through approved range/signing/cache paths on supported browsers and mobile devices.
- [ ] Player controls and transcript/comments meet the accessibility target with keyboard and screen-reader testing.
- [ ] Embed messages validate origin and schema; comments/analytics resist spam, replay, and cross-tenant access.
- [ ] Privacy change or deletion takes effect within issue 21 cache/SLO limits on default and custom domains.

## Required test evidence

- Browser/device/player matrix and range traces.
- Privacy/cache/custom-domain penetration tests.
- Metadata crawler, embed compatibility, accessibility, and analytics-consent reports.

## Risks and open questions

- SSR caches can leak private metadata between requests.
- Signed URL expiry and browser range behavior can interrupt long playback.

## Rollout and rollback

Canary new rendering on alternate routes/custom test domains, then percentage-route public content, then private/embed. Keep a per-link fallback through the observation window.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
