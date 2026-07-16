# Contributing

Work from the dependency map in `_issues/README.md`. Each pull request should name the issue it advances and include the relevant test evidence.

## Local checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p frame-control-plane --target wasm32-unknown-unknown
npx --yes wrangler@4.111.0 deploy --dry-run --config apps/control-plane/wrangler.toml --outdir target/wrangler-dry-run
```

Tests that exercise GStreamer need the native runtime and plugins. Do not add GStreamer dependencies to the Worker or shared domain crates.

Cloudflare Media Transformations cannot be simulated locally. Keep normal local and pull-request tests on the provider-neutral fake; run the real R2 → `MEDIA` → R2 smoke only in the isolated, budgeted remote lane described by issues 09, 10, and 29. Never expose Cloudflare credentials to fork builds.

## Migration rules

- Treat the pinned Cap checkout as a behavior and compatibility reference. Preserve provenance and comply with the applicable upstream license before adapting source.
- Prefer vertical slices that can be observed, compared, and rolled back.
- Keep D1 access in the control plane; native services use authenticated APIs and job contracts.
- Keep Cloudflare Media and native GStreamer behind provider-neutral media ports. Do not leak JavaScript stream/binding types or GStreamer types into shared domain/API contracts.
- Treat the `STREAM` managed video-library binding as a separate capability; the `MEDIA` binding does not authorize adding `[stream]` upload/library/adaptive-playback semantics.
- Never log media content, credentials, signed URLs, session tokens, or unredacted personal data.
- Add a migration and rollback note for persistent-data or object-layout changes.
