# Leptos authenticated web contract v1

Issue 31 replaces the authenticated Next/React surface with a fail-closed
Leptos SSR boundary. The behavioral reference is
`CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`. This document and
`fixtures/web-authenticated/v1/route-matrix.json` define the retained v1 web
surface; they do not attest that production identity, organization, business,
billing, or provider adapters are connected.

## Route and component matrix

The closed matrix contains 17 authenticated routes, three auth routes, three
roles, six render states, three theme preferences, and three viewport fixtures.
Dynamic space and folder identifiers remain path segments; local fixture IDs
are synthetic. Production identifiers may enter only a future typed,
tenant-scoped browser loader after its session boundary is approved.

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
recording list, progress, and revision-fenced form. Controls not backed by an
authority adapter render disabled with an explicit status; they never fabricate
an optimistic resource or success.

## Session and data authority

Every authenticated request starts in one of `loading`, `unauthenticated`,
`denied`, `failed`, or `ready`. Only a server-produced `ready` DTO may render
tenant data. The route performs a second role check before building navigation
or content. A forbidden route renders the same generic denial shell without a
resource identifier, tenant name, recording title, billing fact, or developer
credential. Private HTML is always `no-store` and `noindex,nofollow`.

`AuthenticatedApiPort` is the production adapter boundary. Loads carry a
closed route and normalized query. Mutations carry a closed mutation kind,
expected revision, and redacted idempotency identifier. A mutation declares
the exact cache keys it invalidates. Implementations must authenticate first,
authorize each operation, validate same-origin CSRF, atomically apply the
expected revision and idempotency record, then return only stable error codes
and a safe correlation identifier. Provider errors, tokens, cookies, emails,
object keys, signed URLs, and private titles cannot enter the error DTO.

The Worker contains a tenant-scoped D1 read-model endpoint for direct API
clients, but it is not a web session bootstrap and is not registered as a
Render SSR adapter. ADR 0004 and Issue 42 prohibit Render from receiving or
forwarding browser credentials. Consequently auth POSTs validate bounded
shapes and then return a generic `503`; invalid shapes return `422`. Nonlocal
authenticated routes ignore fixture queries and render `401`. This is
intentional fail-closed behavior, not an authenticated journey.

## Forms, search, and cache behavior

The independent `FormState` contract covers pristine, dirty, invalid, pending,
success, retryable failure, and terminal failure. A submission lease binds the
form revision, rejects duplicate submit while pending, rejects a stale
completion after an edit, preserves an unsaved-change signal, and permits a
retry only after an explicit retryable result. Rendered adapter-pending forms
carry `data-form-contract="revision-fenced-v1"` and
`data-unsaved-guard="required"`; they remain disabled until their server action
exists.

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

Authenticated content is useful without JavaScript: one `h1`, semantic
navigation, labels, status/alert roles, progress semantics, native form
validation hints, a visible skip link on focus, and a `tabindex=-1` main focus
target. The global Leptos hydration island is data-free and therefore cannot
replace the server session decision or flash private content. Hashed hydration
assets are all-or-nothing; missing assets leave the SSR document usable.

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
