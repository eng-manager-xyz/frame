# Share/player v1 local evidence

Captured 2026-07-16. This is **provider-free local evidence only**. It proves
the Rust/SSR/Wasm contract plus offline D1-schema/route conformance and
deliberately does not claim a deployed Worker, hosted D1/R2, custom domain,
CDN, password service, real media, assistive technology, or supported
browser/device execution.

## Reproduction and observed result

```sh
python3 -I scripts/ci/check-share-player.py \
  --evidence target/evidence/share-player-local.json
python3 -I scripts/ci/public-collaboration-sqlite-conformance.py
cargo test -p frame-web --features ssr
cargo clippy -p frame-web --all-targets --features ssr -- -D warnings
cargo check -p frame-web --target wasm32-unknown-unknown \
  --no-default-features --features hydrate --bin frame-web-hydrate
cargo clippy -p frame-web --target wasm32-unknown-unknown \
  --no-default-features --features hydrate --lib -- -D warnings
python3 -I scripts/ci/build-web-hydration.py
python3 -I scripts/ci/check-web-hydration-bundle.py \
  --evidence target/evidence/web-hydration-bundle-local.json
cargo build --locked --release -p frame-web
FRAME_ADDR=127.0.0.1:3817 FRAME_DEPLOYMENT=local \
  FRAME_RELEASE_ID=share-player-local FRAME_WEB_ASSET_DIR=apps/web/dist \
  target/release/frame-web
python3 -I scripts/ci/web-hydration-smoke.py \
  --origin http://127.0.0.1:3817 \
  --evidence target/evidence/web-hydration-smoke-local.json
```

- `check-share-player.py`: pass; 18 authorization cases, eight range cases,
  five compatibility surfaces, five browser/device rows, and eight protected
  evidence identifiers.
- public-collaboration SQLite conformance: pass; nine cross-tenant bindings
  rejected, atomic grant/comment/analytics rate caps enforced, share-bound
  replay isolated, current-transcript uniqueness guarded, and the audit stream
  immutable under migrations through `0022`.
- `cargo test`: 73 tests passed, including 12 focused share-player policy tests,
  SSR metadata/leak tests, exact embed header tests, route-confusion collapse,
  legacy redirect behavior, degraded-mode hydration markup, and consent-first
  collaboration-island markup/scope tests.
- native strict Clippy: pass.
- `wasm32-unknown-unknown` check and strict Clippy: pass for the real hydration
  implementation, including DOM playback controls and the embed listener.
- hydration bundle verification: pass; one CSP-safe bootstrap, JavaScript
  loader, and Wasm module, each named by its full SHA-256 with no source map or
  remote asset.
- local headless Google Chrome smoke: pass for all six hydrated controls,
  labels, pointer/Enter disclosure, focus, the progressively enhanced
  collaboration island, retained SSR content, asset-failure fallback,
  JavaScript-disabled fallback, and a zero-warning/error console.
  The synthetic playback URL intentionally has no media adapter, so this is not
  range, codec, mobile, PiP, fullscreen, or assistive-technology evidence.

The executable tests cover all five privacy policies and lifecycle states;
same-tenant, cross-tenant, owner, explicit, password, anonymous, processing,
terminal, top-level, embed-enabled, and embed-disabled decisions; managed/native
route equivalence; cross-share descriptor rejection; range/suffix/If-Range/416;
legacy and custom host rules; cache revision separation; comment moderation,
rate, replay, and tenant fences; bounded/redacted transcripts; consent-first
analytics; and exact-origin/source/schema/share/sequence embed messages.

## Browser/device and required evidence ledger

The committed browser matrix records the following as
`protected_execution_pending`: Chromium desktop, Firefox desktop, WebKit
desktop, mobile Safari/VoiceOver, and mobile Chromium/TalkBack. No manual or CI
browser run was relabelled as screen-reader, mobile, R2 range, or production
evidence.

| Required evidence | Local status | Production/protected status |
| --- | --- | --- |
| browser/device/player matrix | semantic markup + Wasm compile pass | pending real browsers, devices, media, AT |
| range traces | integer planner unit tests | pending same-origin Worker/R2 wire traces |
| privacy/cache penetration | state collapse + no-store + revision tests | pending CDN/default/custom-domain invalidation SLO |
| metadata crawler | exact SSR assertions | pending external crawler captures |
| embed compatibility | Rust + compiled Wasm origin/source/replay contract | pending reviewed parent sites and browsers |
| accessibility | landmarks, labels, fallback, reduced-motion CSS | pending keyboard, screen reader, VoiceOver/TalkBack reports |
| analytics consent | same-origin hydrated grant/allow/deny/event flow, D1 consent/event operations, retention schema, rate/replay/tenant fences | pending hosted D1 execution, deletion/no-record proof, and supported-browser matrix |
| comments/transcripts | same-origin hydrated load/seek/comment flow, D1 moderation/operation/transcript adapters, bounded mutation/document validators | pending hosted D1 moderation and supported-browser/AT evidence |

## Honest completion boundary

The web process now consumes a bounded, provider-neutral public-share summary
from the configured Worker API through `frame-client`; an unavailable or invalid
upstream still collapses to the generic fail-closed share shell. This does not
provide the missing Instant-specific progress/error projection. The D1 grant,
comment, transcript, consent, analytics, moderation, rate, replay, retention,
and audit adapters are implemented. The hydrated Leptos island now loads
bounded comments/transcript cues, seeks from exact cue positions, submits
idempotent comments, records an explicit consent choice, and emits playback
events only while the in-memory
consent signal remains enabled; its no-JavaScript fallback performs no
mutation. Custom-domain resolution is a pure exact-binding contract, not live
Host routing.
Password verification defines grant semantics but has no password
hashing/delivery adapter. The R2 media route, cache purge, hosted collaboration
journey, supported browser/device/accessibility run, canary, rollback
timing, and observation window remain open. Therefore Issue 32 has substantial
local implementation but no production signoff.
