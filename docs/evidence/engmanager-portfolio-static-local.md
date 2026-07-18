# EngManager portfolio static integration — local evidence

Date: 2026-07-16

Pinned upstream: `matthewharwood/engmanager.xyz@1de52bc8f25793dea3697e67765d53785c05cdfa`

Locally executed against the patched pinned checkout:

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --all-targets`: 78 passed, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- `git diff --check`: passed.
- Frame origin tests cover production, HTTPS preview, loopback development,
  credentials, non-root paths, query/fragment, malformed values, and
  non-loopback HTTP.
- Router tests cover the static homepage CTA, shared navigation link, unchanged
  apex canonical/cache policy, exact cross-origin router/service-worker exits,
  and `421` rejection for every tested Frame-host path.
- Sitemap tests prove every location remains under the apex origin and excludes
  the Frame subdomain.

The portfolio test build generated its ignored root `Cargo.lock` locally. The
patch does not contain it because this static stage adds no dependency. The
issue's exact `frame-client` pin and lockfile correction become mandatory only
if live Frame data is enabled.

Not collected or claimed locally:

- an accepted portfolio PR or portfolio commit;
- keyboard/screen-reader/browser and back-navigation evidence;
- portfolio/Frame preview pairing;
- production DNS/Render/Cloudflare routing, headers, or cache traces;
- a production paired-deployment record.
