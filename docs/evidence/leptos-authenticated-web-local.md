# Local Leptos authenticated-web evidence

This provider-free record covers the locally executable portion of Issue 31.
It proves a closed route/role matrix, SSR privacy boundaries, accessible static
structure, bounded form/query contracts, redirect compatibility, and local
performance. It is not production or epic signoff.

## Reproduction

From the repository root:

```sh
python3 -I scripts/ci/check-web-authenticated-parity.py \
  --evidence target/evidence/web-authenticated-contract-local.json
cargo test --locked -p frame-web
cargo clippy --locked -p frame-web --all-targets -- -D warnings
cargo clippy --locked -p frame-web --no-default-features --features hydrate \
  --target wasm32-unknown-unknown --bin frame-web-hydrate -- -D warnings
python3 -I scripts/ci/build-web-hydration.py
python3 -I scripts/ci/check-web-hydration-bundle.py \
  --evidence target/evidence/web-hydration-bundle-local.json
cargo build --locked --release -p frame-web
FRAME_ADDR=127.0.0.1:3817 FRAME_DEPLOYMENT=local \
  FRAME_RELEASE_ID=web-authenticated-local target/release/frame-web
python3 -I scripts/ci/web-authenticated-parity.py \
  --origin http://127.0.0.1:3817 \
  --evidence target/evidence/web-authenticated-http-local.json
python3 -I scripts/ci/web-authenticated-browser.py \
  --origin http://127.0.0.1:3817 \
  --evidence target/evidence/web-authenticated-browser-local.json \
  --screenshots target/evidence/web-authenticated-screenshots
python3 -I scripts/ci/web-hydration-smoke.py \
  --origin http://127.0.0.1:3817 \
  --evidence target/evidence/web-hydration-smoke-local.json
```

The contract checker validates 17 authenticated routes, 51 role/route cases,
three auth routes, six states, three theme preferences, three breakpoints,
versioned API operation names, rollout flags, and all declared legacy aliases.
Rust tests additionally exercise duplicate submission, retry, stale completion,
unsaved changes, cache invalidation, least privilege, redaction, query bounds,
and open-redirect rejection. The contract gate also proves that the Render app
does not register authenticated SSR or a bearer/tenant-header forwarding path,
as required by ADR 0004 and the Issue 42 capability matrix.

The loopback HTTP report traverses unauthenticated and all owner/admin/member
states for every retained route. It checks status, server-filtered navigation,
generic denial, canonical metadata, `no-store`, `noindex,nofollow`, CSP,
permissions policy, HTML size, query validation, real fixture search/filter,
auth validation/fail-closed behavior, exact legacy redirects, and local SSR
p95. Local fixtures are synthetic and can never be selected in preview or
production.

The Chromium fixture report captures dashboard/owner/dark, library/member/
light, billing/admin-denied/system, account/member/dark, imports/admin/light,
and onboarding/member/system layouts. Across desktop, tablet, and mobile it
checks no horizontal overflow, unique IDs, labels, named controls, 44-pixel
enabled form controls, one current navigation link, a visible three-pixel
focus indicator, and element-level WCAG AA text contrast. The captures are CI
artifacts for review; they are not a cross-browser approved baseline.

## Honest evidence boundary

The following release evidence is absent and remains blocking:

- real production identity/session bootstrap, OTP delivery, signup, recovery,
  logout, and account mutation adapters;
- real organization, library, import, storage, developer, analytics, admin,
  billing, and cache-invalidation adapters;
- a launch-approved browser-side loader using the host-only session contract;
- action-level CSRF/idempotency and N/N-1 client E2E evidence at the web route;
- visual screenshot diffs in current and previous Chrome, Firefox, Safari, and
  Edge at every declared breakpoint/theme;
- named screen-reader and complete manual keyboard walkthroughs;
- protected billing/provider journeys and production edge/API p95 reports;
- owner-approved route flags, timed route-family cutover, and rollback drill.

Until those artifacts exist, all mutation controls remain disabled, auth POSTs
fail closed after validation, nonlocal fixture selection remains impossible,
Render never receives or forwards browser credentials, and legacy route-family
authority must not be removed. The Worker D1 read-model route is only a direct
API building block; it does not close the authenticated browser loader gap.
