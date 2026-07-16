# Contributing

Work from the dependency map in `_issues/README.md`. Each pull request should name the issue it advances and include the relevant test evidence.

## Local checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p frame-control-plane --target wasm32-unknown-unknown
```

Tests that exercise GStreamer need the native runtime and plugins. Do not add GStreamer dependencies to the Worker or shared domain crates.

## Migration rules

- Treat the pinned Cap checkout as a behavior and compatibility reference. Preserve provenance and comply with the applicable upstream license before adapting source.
- Prefer vertical slices that can be observed, compared, and rolled back.
- Keep D1 access in the control plane; native services use authenticated APIs and job contracts.
- Keep media capture and processing behind ports so native platform capture can coexist with GStreamer during migration.
- Never log media content, credentials, signed URLs, session tokens, or unredacted personal data.
- Add a migration and rollback note for persistent-data or object-layout changes.
