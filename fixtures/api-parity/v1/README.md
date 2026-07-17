# API parity v1 fixtures

`route-workflow-report.json` is the machine-readable Issue 30 inventory derived from the ignored,
pinned `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b` checkout. It contains no
customer data, credentials, request bodies, or upstream source code. Each source identity is a path,
symbol, and SHA-256 only.

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
fallback availability is false. A separate typed static allowlist promotes only the source-pinned
`cap-v1-05b6ba3f76daac22` `GET /api/status` contract, with exact path/method/request/response tests
and no D1 dependency. The other 287 rows fail closed without pretending an external fallback or
business handler exists.

Regenerate only from the exact pinned checkout:

```sh
python3 scripts/ci/check-api-workflow-parity.py --generate --require-reference
```

Normal CI runs the checker offline. If the reference or scope changes, create a new versioned corpus
instead of silently rewriting v1.
