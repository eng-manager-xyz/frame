# Leptos authenticated web contract v1

Issue 31 replaces the authenticated Next/React surface with a data-free Leptos
SSR shell and a browser-direct typed client. The behavioral reference is
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`. This document and
`fixtures/web-authenticated/v1/route-matrix.json` define the retained v1 web
surface. The local contract includes the Worker session verifier and durable
action boundary; it does not attest that production identity issuance,
provider execution, billing, or route cutover is approved.

## Route and component matrix

The closed matrix contains 17 authenticated routes, three auth routes, three
roles, six render states, three theme preferences, and three viewport fixtures.
The browser role set is exactly owner/admin/member; a legacy `viewer`
membership is denied before any workspace DTO is built.
Dynamic space and folder identifiers remain path segments; local fixture IDs
are synthetic. Production identifiers enter only the typed browser loader
after the Worker authenticates the host-only session and derives an active
membership; they never enter Render SSR.

| Family | Canonical routes | Reusable primitive | Roles |
|---|---|---|---|
| auth | `/login`, `/signup`, `/verify` | bounded POST form, alert, retry link | public pending challenge |
| home/library | `/dashboard`, `/library` | workspace layout, search/filter, recording list, empty state | owner/admin/member |
| organization | `/spaces`, `/spaces/{space_id}`, `/folders`, `/folders/{folder_id}` | collection and detail boundaries | owner/admin/member reads; owner/admin creates |
| onboarding/import | `/onboarding`, `/imports` | revision-fenced form, durable progress | onboarding all roles; imports owner/admin |
| settings | `/settings`, `/settings/account`, `/settings/organization`, `/settings/members`, `/settings/storage` | settings index, definition list, form/status boundary | account all roles; tenant controls owner/admin |
| restricted | `/developer`, `/analytics`, `/admin`, `/billing` | server-authorized restricted panel | owner/admin except billing owner only |

The reusable design primitives are semantic Rust/Leptos builders rather than a
second client-side authorization model: document metadata, skip link and focus
target, private-state shell, workspace navigation, panel/notice/empty states,
recording list, progress, and revision-fenced form. The hydrated action panel
becomes enabled only after an authorized Worker DTO arrives; it never
fabricates an optimistic resource or success.

## Session and data authority

Every authenticated browser request starts in one of `loading`,
`unauthenticated`, `denied`, `failed`, or `ready`. Only a Worker-produced
`ready` DTO may render tenant data. The typed client performs a second closed
role check before building content. A forbidden route renders the same generic denial shell without a
resource identifier, tenant name, recording title, billing fact, or developer
credential. Private HTML is always `no-store` and `noindex,nofollow`.

`BrowserAuthenticatedClient` is the production browser adapter. Loads carry a
closed surface and normalized query to `/api/v1/web/workspace/{surface}`.
Each workspace DTO also carries the user's organization-selection revision and
an opaque digest context bound to that user, selected organization, and
revision. Mutations echo that context and carry one of eleven closed actions,
an expected organization revision, bounded input, and a redacted idempotency identifier to
`/api/v1/web/actions/{action}`. A receipt declares the exact cache domains it
invalidates and an exact `effect_state`. Space/folder creation, import queueing,
and account/organization updates return HTTP `200`/`applied`. Onboarding, membership,
storage, developer-key, billing, and admin requests return
HTTP `202`/`pending_protected_execution`: the durable intent exists, but neither the
Worker nor UI claims that the protected provider/product change occurred.
Unknown response fields, roles, states, cache domains, action
names, and oversized DTOs fail closed. Provider errors, tokens, cookies,
emails, object keys, signed URLs, and private titles cannot enter the error DTO.

The Worker admits no bearer or `x-frame-tenant-id` header on these browser
routes. It reads `__Host-frame_session`, reuses `AuthService` and
`D1AuthStateRepository`, requires `Sec-Fetch-Site: same-origin`, and derives the
tenant only from `users.active_organization_id` plus
`organization_preference_revision`. The selected organization must have an
active owner/admin/member membership; an absent, invalid, or stale selection is
denied without falling back to the oldest membership. The first DTO query
binds the exact selected organization, selection revision, membership role,
and membership revision, and the Worker revalidates the same authority after
all DTO reads before returning private data. Mutations additionally
require the exact request Origin and matching `__Host-frame_csrf`/
`x-frame-csrf` values. The repository-minted one-use mutation grant is deleted
in the same D1 batch as the organization revision assertion, the exact
active-organization selection and selection-revision assertion, the exact
membership role and membership-revision assertion, a still-current active
session/grant assertion, the local product effect or provider-neutral pending
intent, idempotency operation, and stable receipt.
Every changed-row postcondition is asserted before commit. A retry with
identical input replays the receipt only after the replay batch reasserts the
current selection, membership role/revision, session, and one-use grant; key
reuse with different input conflicts.

`FRAME_AUTH_HASH_KEYRING_V1` is a Worker secret containing a deny-unknown-field
JSON object with `active` and optional `fallback` keys. Each key has a nonzero
`version` and 32–128 bytes of lowercase hex `material_hex`. The session issuer
and verifier must use the same active/fallback window. Missing, malformed, or
weak configuration makes browser authentication unavailable; it never falls
back to an API-key digest or deterministic material.

ADR 0004 and Issue 42 still prohibit Render from receiving or forwarding
browser credentials. Nonlocal authenticated routes therefore SSR a generic
`401`/no-store shell. After hydration, the browser replaces only the dedicated
island and calls the same-origin Worker directly. Auth issuance POSTs remain a
separate fail-closed dependency until their production adapters are approved.

## Forms, search, and cache behavior

The independent `FormState` contract covers pristine, dirty, invalid, pending,
success, retryable failure, and terminal failure. A submission lease binds the
form revision, rejects duplicate submit while pending, rejects a stale
completion after an edit, preserves an unsaved-change signal, and permits a
retry only after an explicit retryable result. The no-JavaScript/server fixture
forms carry `data-form-contract="revision-fenced-v1"` and
`data-unsaved-guard="required"`. The browser island enforces one pending
submission, generates one fresh idempotency identifier per new logical
mutation, reports validation and conflict states, and invalidates every cached
workspace envelope before a valid mutation crosses the transport boundary.
If the response is lost or invalid, it hides stale data, force-refreshes the
authorized workspace where possible, and retains the exact body and key for an
explicit safe replay. All route envelopes share the organization
revision and organization/member collections, so receipt-domain-only eviction
would retain stale authority or data. It
announces `applied` only for a committed local effect; pending protected work is
announced as recorded with no provider change claimed.

Library query parameters are closed to `q` (120 bytes, no control/markup
characters), `filter=all|ready|processing|failed`, `page=1..1000`, and
`theme=system|dark|light`. Invalid values fail with `400` rather than silently
changing a query. The fixture implements real server-side search/filter and
preserves normalized values. Production pagination cursors must remain opaque;
the numeric page is a compatibility view parameter, not database authority.

## URL, metadata, and redirect compatibility

Private canonical links exclude fixture, search, filter, theme, and page query
parameters. Auth material is never accepted as a redirect target. A safe
post-auth path is a retained same-origin absolute path with no scheme,
authority, query, fragment, control character, backslash, or parent segment.

Legacy `/dashboard/...` aliases for settings, imports, developer, billing,
analytics, spaces, folders, and dynamic space/folder details return `308` to an
exact canonical path. Alias queries are intentionally discarded so OTPs,
OAuth codes, emails, and fixture selectors cannot be propagated. Unknown
legacy behavior still requires an explicit retirement disposition under the
migration charter; an accidental 404 is not parity.

## SSR, hydration, accessibility, and themes

The generic authenticated shell is useful without JavaScript: one `h1`,
status/alert roles, a visible skip link on focus, and a `tabindex=-1` main focus
target. It intentionally cannot display private data or mutate without
JavaScript because ADR 0004 assigns authenticated loading to the browser. The
global Leptos hydration island is data-free and cannot flash private content.
Hashed hydration assets are all-or-nothing; missing assets leave a safe sign-in
or unavailable shell rather than a partially authorized document.

System, explicit dark, and explicit light preferences share the same semantic
tree. Responsive layouts cover 390×844, 768×1024, and 1440×1000 fixtures, with
single-column navigation/forms on narrow viewports and reduced-motion rules.
The existing Chromium hydration smoke proves focus visibility, keyboard and
pointer operation, no-JavaScript fallback, and no browser diagnostics. The
authenticated browser gate additionally retains six synthetic visual captures
and checks responsive overflow, unique IDs, labels, control size, current-page
navigation, focused skip-link visibility, and element-level WCAG AA contrast.
Named screen-reader walkthroughs, screenshot diffs across the full browser
charter, and complete manual keyboard checks remain protected/manual release
evidence.

## Budgets, rollout, and rollback

The matrix pins the charter's local server-render p95 budget at 750 ms and API
p95 at 500 ms. It additionally limits private HTML to 128 KiB, generated
JavaScript to 500,000 bytes, and Wasm to 2,000,000 bytes. The loopback E2E gate
measures every state/role response; protected edge/API p95 evidence is still
required before cutover.

Each route row owns a `web.*.v1` tenant flag. Promotion is by route family only
after its real adapter, owner/admin/member/denial E2E cases, accessibility,
visual, and performance evidence pass. Rollback disables that family and sends
legacy deep links to the approved legacy handler. It does not roll back D1/R2,
delete data, accept conflicting writes, or reinterpret an acknowledged action.
See `docs/operations/leptos-route-cutover.md`.
