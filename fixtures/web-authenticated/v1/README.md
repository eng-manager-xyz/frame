# Authenticated web parity fixtures v1

`route-matrix.json` is the closed Issue 31 route, role, state, theme, breakpoint,
and rollout inventory. It contains synthetic identifiers only. The Rust tests
bind every row to `AuthenticatedRoute`; the Python checker binds every fixture
path and legacy alias to the Axum router and verifies that protected evidence
remains visibly pending.

`browser-direct-boundary.json` closes the ADR-0004 credential topology and
partitions the eleven mutations into locally applied effects versus explicit
`pending_protected_execution` intents. The companion SQLite fault suite proves
that active-organization selection/revision, organization revision, membership
role/revision, current session/grant, effect, receipt, and grant consumption
form one rollback-safe batch. Completed receipt replay reasserts the same
current authority before consuming its one-use grant. Workspace loads bind the
exact selection and membership both before and after DTO reads. An uncertain
mutation outcome discards every cached envelope, refreshes authority where
possible, and retains the exact request and idempotency key for safe replay.

Local `?fixture=owner|admin|member|empty|loading|denied|failed` values are visual
and contract inputs, not sessions. Nonlocal deployments ignore them and render
the unauthenticated boundary.
