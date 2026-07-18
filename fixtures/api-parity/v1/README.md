# API parity v1 fixtures

`route-workflow-report.json` is the machine-readable Issue 30 inventory derived from the ignored,
pinned `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b` checkout. It contains no
customer data, credentials, request bodies, or upstream source code. Each source identity is a path,
symbol, and SHA-256 only.

Each row also has an exact completion decision. It distinguishes unfinished local adapter or
retirement-response work from overlapping provider, hardware, or human-approval gates and records
the production fail-closed behavior. The checker rejects any attempt to use a protected gate to
erase pending repository-local work.

`operation-contract-catalog.json` is the normalized specification backlog for those same 288
identities. Fifty-four source-manifest-bound profiles describe the required success-or-retirement,
validation, authorization, idempotency/retry, and stable-failure cases without duplicating policy
text in every row. Its role is explicitly `specification_only_not_endpoint_execution_evidence`:
the checker will not let a profile promote an adapter, clear remaining local work, or manufacture a
retirement approval. The route report remains the authority for what has actually passed.

`contract-cases.json` freezes Frame's closed public errors, provider-neutral media statuses, and
current/N-1 compatibility decisions. Rust decodes and validates it in
`crates/domain/tests/api_workflow_contract_v1.rs`.

`frame-application::LegacyCompatibilityRegistryV1` also decodes the complete report in its focused
suite. With explicit synthetic fallback availability, it proves every unpromoted row chooses
fallback, exercises shared admission and the atomic execution-port boundary for every retained
row, and covers current/previous decisions for all release-managed client associations. This
registry evidence does not change a row's endpoint success state; per-operation business adapters,
released-client binaries, providers, and approved retirements remain separate gates.

The control-plane's fail-closed transport constructs the registry and implements that port with a
digest-only D1 claim/intent/completion/audit journal. Local SQLite conformance proves its atomic
and immutable SQL behavior. Its durable semantic-adapter allowlist is empty and production
fallback availability is false. A typed semantic allowlist promotes only the source-pinned
`cap-v1-05b6ba3f76daac22` `GET /api/status`, `cap-v1-ff19008f47194c43`
`GET /media-server`, `cap-v1-a1b180c5d123c870` `GET /api/changelog/status`, and
`cap-v1-16668b858461f386` `OPTIONS /api/changelog/status`, plus
`cap-v1-0fa8384f3666825b` `GET /api/changelog` and `cap-v1-237f41f3086a2d67`
`OPTIONS /api/changelog` contracts, `cap-v1-4f21920a947c4c84`
`GET /api/mobile/session/config`, and the actor-bound D1
`cap-v1-d130c840f654bd72` `GET /api/notifications/preferences` contract. These eight semantic
adapters carry all five per-operation evidence axes and exact path/method/query/request/response/header
tests. All production ingresses enforce their reported bucket through the bounded, keyed-digest D1
authority in migration `0034_compatibility_rate_limits.sql`; only the notification preference read
also requires D1 business data. SQLite conformance proves exact fixed-window saturation/reset,
bucket and subject separation, bounded cleanup, regressed-clock rejection, and fail-closed behavior
when the authority is absent. The exact
`cap-v1-a3b4c805d409bc7c` active-organization ACTION is the ninth locally proven contract. The
other 279 rows fail closed without
pretending an external fallback or business handler exists.

`changelog-feed.json` is the exact 88,817-byte `JSON.stringify` response derived from the pinned 99
numeric MDX inputs. The generator verifies every input hash, a deterministic source-manifest digest,
and the response digest before writing or accepting the fixture.

Regenerate only from the exact pinned checkout:

```sh
python3 scripts/ci/check-api-workflow-parity.py --generate --require-reference
```

Normal CI runs the checker offline. If the reference or scope changes, create a new versioned corpus
instead of silently rewriting v1.
