# API parity v1 fixtures

`route-workflow-report.json` is the machine-readable Issue 30 inventory derived from the ignored,
pinned `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b` checkout. It contains no
customer data, credentials, request bodies, or upstream source code. Each source identity is a path,
symbol, and SHA-256 only.

`contract-cases.json` freezes Frame's closed public errors, provider-neutral media statuses, and
current/N-1 compatibility decisions. Rust decodes and validates it in
`crates/domain/tests/api_workflow_contract_v1.rs`.

Regenerate only from the exact pinned checkout:

```sh
python3 scripts/ci/check-api-workflow-parity.py --generate --require-reference
```

Normal CI runs the checker offline. If the reference or scope changes, create a new versioned corpus
instead of silently rewriting v1.
