# Authenticated web parity fixtures v1

`route-matrix.json` is the closed Issue 31 route, role, state, theme, breakpoint,
and rollout inventory. It contains synthetic identifiers only. The Rust tests
bind every row to `AuthenticatedRoute`; the Python checker binds every fixture
path and legacy alias to the Axum router and verifies that protected evidence
remains visibly pending.

`browser-direct-boundary.json` closes the ADR-0004 credential topology and
partitions the twelve mutations into locally applied effects versus explicit
`pending_protected_execution` intents. The companion SQLite fault suite proves
that each action's organization or user-selection revision, applicable
membership authority, current session/grant, effect, receipt, and grant
consumption form one rollback-safe batch. Completed receipt replay reasserts
the applicable current authority before consuming its one-use grant. Tenant
workspace loads bind the exact selection and membership both before and after
DTO reads; a dangling selection yields only bounded membership choices for
recovery. The SQLite suite executes the native Frame UUID selector from absent
and stale selections, fences target membership and selection/session races,
proves rollback atomicity, and proves completed replay cannot duplicate the
write, return a stale selection receipt, reuse its consumed grant, or reuse the
same key for a different target. Exact
pinned Cap Navbar invalidate-then-void
server-action ingress remains an Issue 30 local-contract gap. An uncertain
mutation outcome discards every cached envelope, refreshes authority where
possible, and retains the exact request and idempotency key for safe replay.

Local `?fixture=owner|admin|member|empty|loading|denied|failed` values are visual
and contract inputs, not sessions. Nonlocal deployments ignore them and render
the unauthenticated boundary.
