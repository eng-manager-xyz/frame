# Cross-repository contract and preview strategy

Frame owns canonical versioned DTO fixtures and schema under
`fixtures/frame-api/v1`; it never mutates the portfolio repository. The
portfolio owns its dependency pin, lockfile, rendering fixtures, deployment,
and release decision. A fixture copy must record the Frame source SHA and fail
on drift.

The executable definition is
`fixtures/cross-repo-preview/v1/ci-policy.json`; its checker verifies the exact
fixture digests, five producer/consumer lanes, distinct toolchain/cache owners,
protected evidence classifications, and workflow wiring. The six seeded cases
in `compatibility-cases.json` make an additive field pass while required-field,
major, type, status, and public-path changes fail the current Rust consumer.
The last released portfolio consumer must reach the same result in its
protected checkout before that evidence exists.

| Producer | Consumer | Required cadence | Authority |
| --- | --- | --- | --- |
| Frame candidate | Frame current client/Worker/web | every Frame PR | Frame quality gate |
| Frame candidate | last released portfolio consumer | every public-contract Frame PR | portfolio checkout using its pinned toolchain; protected cross-repo evidence |
| released Frame fixture/SHA | portfolio candidate | every portfolio PR | portfolio unprivileged CI |
| Frame main | portfolio default branch | scheduled advisory | read-only check; never updates a pin |
| current plus N-1 API | current plus N-1 web/client | every release | Frame production preflight and staged provider compatibility |

Additive fields/capabilities must pass older consumers. Seeded major, type,
status, privacy, and path changes must fail before release. Breaking changes
ship as a parallel major and remain until the pinned portfolio has moved.

## Local topology

`scripts/frame preview-e2e` starts two independent loopback origins and a
Worker-semantic fake using only standard-library Python and synthetic fixtures.
The portfolio process polls Frame in a bounded background task and serves only
last-known-good state; request handlers never wait for Frame. The journey checks
top-level navigation/back, cookie separation, health/share/error fixtures,
generic unavailable states, ranges/captions, privacy revocation, dynamic cache
bypass, immutable-asset hit behavior, malformed/incompatible/slow/outage
degradation, and cleanup. Five named contract defects plus a retained-sensitive-
audit defect must each fail at the expected invariant. Frame restart on the
same paired origin proves bounded background retry, rollback recovery, and
reconnect without coupling a portfolio request handler to Frame.

`.github/workflows/cross-repository-contract.yml` owns the Frame-side producer
and current-consumer gate. Its external job is reachable only by a weekly
advisory or protected manual dispatch, checks out the recorded portfolio SHA
without persisted credentials, honors the portfolio nightly and lockfile, and
invokes a fixed `frame_contract` Cargo target against the Frame candidate
fixtures. A missing portfolio workflow, lockfile, provenance record, or test
target fails; Frame never patches or pushes the other checkout.

Issue 37's checked-in portfolio artifact is a static-link-only patch at the
same base SHA. The policy reads its manifest and requires `live_frame_data` and
`frame_client_dependency` to remain false. Its local Rust/router/golden result
is valuable integration evidence, but it is not the last-released portfolio
contract consumer and cannot satisfy that protected row.

This semantic harness is not a browser or provider emulator. Real Chrome/WebKit/
Firefox, screen readers, Cloudflare routing/cache, Render, R2, Media, and the
portfolio build remain separate evidence classes.

## Preview topology

A trusted operator pairs a manual, three-day Render Frame preview with the
fixed staging API origin. It is noindex and receives no production secret.
Authenticated staging additionally requires isolated D1/R2 names, exact preview
origin policy, synthetic data, and a cleanup owner. The portfolio preview link
records both commits and preview identifiers. Untrusted PRs cannot create,
mutate, extend, or clean provider previews.

Provider canaries are protected and bounded. Canonical public smoke can run
without secrets; upload/finalize/media/cache-purge checks use isolated staging
credentials, generated media, a cost cap, concurrency one, explicit cleanup,
and a kill switch. A provider retry retains the first failure.
