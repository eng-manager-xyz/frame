# Local web SSR and hydration evidence

This provider-free check covers the web portions of issues 08, 31, and 32 that
can be proven without an authenticated backend, production media, or protected
infrastructure. It does not claim page-family parity or production closure.

## Scope and authority

- Axum renders useful landmarks, route metadata, privacy state, loading/error
  state, native video controls, captions, and no-JavaScript help.
- Two exact Leptos subtrees hydrate: a data-free readiness boundary on every
  page and public-player keyboard help. The components share one Rust source
  between SSR and Wasm and start from identical markup.
- Authentication, role, private workspace, playback availability, and metadata
  remain server-authoritative. Hydration cannot invent authenticated success.
- The public fixture exists only under local deployment. Unknown, processing,
  and unauthenticated states remain generic and non-cacheable.
- The browser smoke rejects hydration exceptions, warnings, and console errors;
  it excludes only the two exact local fixture media URLs because this slice
  intentionally has no production playback backend.

## Reproduction

```sh
cargo test --locked -p frame-web
cargo clippy --locked -p frame-web --all-targets -- -D warnings
cargo clippy --locked -p frame-web --no-default-features --features hydrate \
  --target wasm32-unknown-unknown --bin frame-web-hydrate -- -D warnings
python3 -I scripts/ci/build-web-hydration.py
python3 -I scripts/ci/check-web-hydration-bundle.py \
  --evidence target/evidence/web-hydration-bundle-local.json
cargo build --locked --release -p frame-web
FRAME_ADDR=127.0.0.1:3817 FRAME_DEPLOYMENT=local \
  FRAME_RELEASE_ID=web-hydration-local target/release/frame-web
python3 -I scripts/ci/web-hydration-smoke.py \
  --origin http://127.0.0.1:3817 \
  --evidence target/evidence/web-hydration-smoke-local.json
```

The local browser walkthrough additionally verifies the disclosure by pointer
and Enter key, the collapsed `aria-expanded=false` state, main/navigation/
heading landmarks, visible focus, responsive layout, and an empty warning/error
console. CI repeats the real Chromium smoke and retains both JSON records.

## Bundle and rollback

The bundle contains only a manifest and three full-SHA-256-named local assets:
the CSP-safe bootstrap, generated JavaScript loader, and Wasm module. Axum
verifies all hashes and serves only those allowlisted names with immutable
caching. A missing, partial, oversized, or tampered bundle removes the module
preload and script atomically; SSR/no-JavaScript content is the rollback.

Release packaging places the verified directory at `web-dist` next to the
binary. The production workflow launches that package from `/tmp` to prove it
does not depend on the repository working directory. Render independently
builds the same pinned Trunk input and copies it beside the Rust binary.
Production/preview never search the working-directory checkout, and their
readiness endpoint returns a non-success response if the packaged assets are
missing or invalid. Local SSR-only fallback remains available deliberately.
