# Authenticated web parity fixtures v1

`route-matrix.json` is the closed Issue 31 route, role, state, theme, breakpoint,
and rollout inventory. It contains synthetic identifiers only. The Rust tests
bind every row to `AuthenticatedRoute`; the Python checker binds every fixture
path and legacy alias to the Axum router and verifies that protected evidence
remains visibly pending.

Local `?fixture=owner|admin|member|empty|loading|denied|failed` values are visual
and contract inputs, not sessions. Nonlocal deployments ignore them and render
the unauthenticated boundary.
