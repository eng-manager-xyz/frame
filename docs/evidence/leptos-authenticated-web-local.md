# Local Leptos authenticated-web evidence

This provider-free record covers the locally executable portion of Issue 31.
It proves a closed route/role matrix, SSR privacy boundaries, accessible static
structure, bounded form/query contracts, the browser-direct Worker session and
action boundary, CSPRNG issuance, fixed-shape authenticated delivery sealing,
ciphertext-only D1 handoff, redirect compatibility, and local performance. It
is not production or epic signoff.

## Reproduction

From the repository root:

```sh
python3 -I scripts/ci/check-web-authenticated-parity.py \
  --evidence target/evidence/web-authenticated-contract-local.json
cargo test --locked -p frame-web
cargo test --locked -p frame-control-plane browser_web_runtime
cargo test --locked -p frame-control-plane worker_auth_runtime
python3 -I scripts/ci/web-authenticated-action-sqlite-conformance.py \
  --evidence target/evidence/web-authenticated-action-sqlite-local.json
python3 -I scripts/ci/worker-auth-sqlite-conformance.py \
  --evidence target/evidence/worker-auth-sqlite-local.json
cargo clippy --locked -p frame-control-plane --all-targets -- -D warnings
cargo clippy --locked -p frame-control-plane \
  --target wasm32-unknown-unknown -- -D warnings
worker-build --release apps/control-plane
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
four auth routes, six states, three theme preferences, three breakpoints,
versioned API operation names, rollout flags, and all declared legacy aliases.
Rust tests additionally execute all 51 route/role loads and every permitted or
denied action combination across the twelve typed actions. They exercise
duplicate submission, exact-key retry after an uncertain response, stale
completion, unsaved changes, complete workspace-cache invalidation on both
confirmed and uncertain mutation outcomes, least privilege, redaction, query
bounds, and open-redirect rejection. Six actions return HTTP `200` with an exact locally `applied`
effect; the other six return HTTP `202`/`pending_protected_execution` after recording a bounded durable intent,
and the UI explicitly says that no provider change is claimed. The contract
gate proves that Render has no authenticated SSR or
credential-forwarding path and that the browser client can call only relative
same-origin `/api/v1/web/*` paths.

Seven focused Worker-auth tests prove fresh URL-safe session/CSRF/API/OAuth
material, unbiased six-digit OTP shape, strict versioned nonzero 256-bit AEAD
keys, fixed 1,071-byte AES-256-GCM delivery ciphertext, plaintext absence,
randomized nonces, encrypted pending-cookie round trips, key rotation/tamper/
expiry rejection, extreme-timestamp rejection, canonical base64url, exact
single-decode form fields, and host-only cookie attributes. The exact login,
signup, recovery, verify, and logout routes reuse `AuthService` and
`D1AuthStateRepository`; canonical Leptos forms post directly to the Worker,
not Render. The native and `wasm32-unknown-unknown` strict gates plus the
release `worker-build` prove the crypto and CSPRNG dependency closure compiles
for Cloudflare Workers.

Migration 0029 adds a ciphertext-only provider handoff after the existing
fenced auth outbox. The SQLite suite rejects forged initial `delivering` and
`delivered` rows, proves insert/ack crash replay deduplicates by delivery ID, a
conflicting ciphertext digest cannot replace the first handoff, payloads are
immutable, provider attempts/transitions are fenced, and terminal receipts
cannot reopen or mutate. This is local durability evidence only: the suite
records `provider_execution=protected_not_executed` and never claims an email
or SMS was sent.

The Worker tests and static boundary fixture prove reuse of
`auth_sessions_v2` through `D1AuthStateRepository`, host-only session admission,
tenant data selection exclusively from `users.active_organization_id` and its
preference revision, membership-only dashboard recovery when that selection is
dangling, exact selection/membership binding in the first tenant DTO query
and final authority revalidation after its reads, exact mutation Origin/Fetch Metadata,
double-submit CSRF, bounded deny-unknown-field bodies, stable idempotency, and
atomic consumption of the repository-minted one-use mutation grant. D1
migration 0025 couples the action-specific organization or user-selection
revision, applicable membership authority, local product effect or pending intent, action receipt, changed-row
postconditions, and grant deletion in one batch. The SQLite fault suite changes
the selected organization, selection revision, role, membership state,
membership revision, and session state after
precheck/validation and proves each stale-authority batch rolls back without an
operation, effect, product row, revision increment, or grant deletion. `viewer`
is outside the closed
browser role set and is denied before a DTO is loaded. No browser tenant or
bearer header is accepted, and Render remains outside the credential path.
An unknown mutation response cannot retain a workspace envelope or mint a new
logical operation: the island hides stale data, refreshes where possible, and
retries the identical body and idempotency key.
The same suite proves an absent or stale selection cannot expose another
tenant's data; only bounded, active owner/admin/member choices are returned.
It executes the native Frame UUID selector from both recovery states and proves
the prior nullable selection/revision, target membership, user update, effect,
completed receipt, and one-use grant consumption commit together. Races in the
selection value/revision, target membership/role, or session roll the whole
selector batch back. A completed selector retry reuses its one durable receipt,
does not increment the selection twice, rechecks the receipt's exact target
selection/revision and target membership, and consumes a fresh grant exactly
once. Sequential reuse of that key with a different target conflicts without
creating a second operation or effect. Ordinary completed-action replay also
rejects a concurrent selection change, demotion, membership removal, or session
revocation before consuming its grant.

This evidence covers the native Frame UUID selector only. It does not implement
or claim exact parity for the pinned Cap Navbar invalidate-then-void server-action
ingress; that remains the explicit Issue 30 local-contract gap.

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

- hosted D1 identity/session authority, provisioned hash/delivery/pending key
  windows, real email/SMS OTP delivery, and deployed login/signup/recovery/
  logout journey receipts;
- authoritative onboarding, membership, storage, developer, billing, admin,
  and other protected execution beyond the durable local pending-intent
  boundary;
- N/N-1 deployed Worker/web compatibility and production session journey
  evidence at the browser route;
- visual screenshot diffs in current and previous Chrome, Firefox, Safari, and
  Edge at every declared breakpoint/theme;
- named screen-reader and complete manual keyboard walkthroughs;
- protected billing/provider journeys and production edge/API p95 reports;
- owner-approved route flags, timed route-family cutover, and rollback drill.

Until those artifacts exist, local auth issuance may commit a sealed D1
handoff but no provider message is claimed or sent, nonlocal fixture selection
remains impossible, Render never receives or forwards browser credentials,
provider effects cannot be claimed from a durable intent alone, and legacy
route-family authority must not be removed. The hydrated mutation control
appears only after a successful Worker DTO; the SSR fallback never assumes a
session or enables an action.
