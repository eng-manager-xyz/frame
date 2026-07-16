---
title: "Reach Rust API parity for desktop, mobile, dashboard, share, storage, developer, webhook, and admin workflows"
labels:
  - "phase:p5"
  - "area:api"
  - "area:rust"
  - "area:security"
  - "type:migration"
  - "risk:high"
depends_on: [07, 12, 13, 14, 15, 18, 19, 28]
size: epic
---

# 30 · Reach Rust API parity for desktop, mobile, dashboard, share, storage, developer, webhook, and admin workflows

## Outcome

Supported Cap clients and Leptos surfaces can use versioned Rust APIs for every retained workflow without hidden TypeScript authority.

## Current Cap reference

The pinned Cap web app exposes dozens of API route entrypoints, many server actions, Effect/Hono backend services, desktop/mobile/extension APIs, developer endpoints, webhooks, cron jobs, and durable workflows.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#07](./07-p1-control-plane-media-job-protocol.md), [#12](./12-p2-d1-repositories-query-conformance.md), [#13](./13-p2-auth-sessions-identity.md), [#14](./14-p2-organizations-rbac-spaces-folders.md), [#15](./15-p2-video-collaboration-business-data.md), [#18](./18-p3-object-storage-adapter-key-contract.md), [#19](./19-p3-multipart-upload-download.md), [#28](./28-p4-media-service.md)

## Scope

Inventory and port retained routes/actions/workflows: auth/session, videos, upload, storage, folders/spaces/orgs, comments/notifications, share/embed, desktop/mobile/extension, developer APIs, billing, imports, media callbacks, webhooks, scheduled jobs, admin/support, analytics consent, and external integrations.

### Out of scope

Pixel/UI parity is issues 31–33. Third-party vendors can remain adapters if contracts and security meet the charter.

## Deliverables

- [ ] Route/action/workflow matrix with legacy path, clients, auth, policy, contract version, owner, implementation, and disposition.
- [ ] Rust handlers/services with centralized validation, authorization, rate limits, idempotency, errors, tracing, and audit.
- [ ] Signed webhook verification, replay defense, outbound retry/outbox, cron/workflow lease, and secret rotation patterns.
- [ ] Compatibility layer for supported desktop/mobile/extension/developer clients and deprecation policy.
- [ ] Generated API documentation and contract/E2E suites.

## Acceptance criteria

- [ ] Every in-scope route, server action, and workflow has a passing success, validation, authorization, idempotency/retry, and failure contract or approved retirement.
- [ ] Unknown IDs and forbidden cross-tenant resources follow the approved non-disclosure policy.
- [ ] Webhook signatures, replay windows, rate limits, body limits, SSRF/open-redirect defenses, and secret redaction pass security tests.
- [ ] Scheduled and durable workflows survive duplicate execution, crash, timeout, and partial provider failure.
- [ ] Supported legacy clients pass compatibility E2E tests before any endpoint is redirected.

## Required test evidence

- Machine-readable route parity report.
- Contract tests against Cap snapshots and Frame.
- Security, load, retry, and workflow fault report.

## Risks and open questions

- A route-count checklist can miss server actions and side effects.
- Compatibility shims can become permanent unless deprecation dates are enforced.

## Rollout and rollback

Strangle one route/workflow family at a time with shadow traffic and per-client flags. Preserve a route-level fallback until parity and SLO observation complete.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
