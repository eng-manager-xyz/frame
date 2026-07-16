# Local development

The local stack is isolated from production by configuration and by the
command guard in `scripts/frame`. It uses local Wrangler D1/R2 state and the
deterministic Media fake; the production-only remote Media binding is absent.
No checked-in value can address the production database or bucket.

## First run

```sh
scripts/frame doctor
scripts/frame reset
scripts/frame start
```

The web service listens on `http://127.0.0.1:3000`; the Worker listens on
`http://127.0.0.1:8787`; and the persistent native media health service listens
on `http://127.0.0.1:8790`. `Ctrl-C` stops all three, and `scripts/frame stop` is
safe to run after an interrupted shell. Logs are kept under ignored `.tmp/` and
can be tailed with `scripts/frame logs`.

`reset` deletes only `apps/control-plane/.wrangler/state` inside this checkout, reapplies every
migration, and loads synthetic `*.invalid` seed identities. It refuses to run
when `FRAME_DEPLOYMENT=production`, `CLOUDFLARE_API_TOKEN`, or `DATABASE_URL` is
present. It never contacts or deletes a remote bucket.

The seed installs a hashed API-key record for the synthetic local owner. The
raw local-only credential is `frame-local-api-key-test-only-0000000001`; send it
as a Bearer token together with
`x-frame-tenant-id: 018f47a6-7b1c-7f55-8f39-8f8a86900102`. The Worker hashes the
credential before D1 lookup and independently requires an active organization
membership and an allowed `frame:read`, `frame:write`, or `frame:admin` scope.
This credential exists only in the disposable local seed and is never a
production fallback.

## Focused commands

```sh
scripts/frame check
scripts/frame test
scripts/frame migrate
scripts/frame seed
scripts/frame media-smoke
```

The Media Transformations binding cannot be faithfully emulated. Offline
development tests the same capability, idempotency, output-key, fallback, and
publication contracts through the fake and native GStreamer. Real R2-to-Media-
to-R2 checks run only in the protected CI environment with separate staging
resources, hard cost/time limits, synthetic fixtures, and explicit cleanup.

## Troubleshooting

- Missing `wasm32-unknown-unknown`: run
  `rustup target add wasm32-unknown-unknown`.
- Missing GStreamer: install the runtime, development headers, and base/good
  plugin sets for the OS, then rerun `scripts/frame doctor`.
- Missing Worker tooling: install `worker-build 0.8.5` and `cargo-deny 0.20.2`
  with Cargo's `--locked` flag. The doctor rejects other versions.
- Missing browser tooling: install Chromium/Chrome/Edge, or enable
  `safaridriver` on macOS. Tauri CLI is reported separately because it is only
  required while developing the native shell.
- Port collision: set `FRAME_ADDR=127.0.0.1:<port>` for the web service or pass
  a different Wrangler port while running the components separately. Set
  `FRAME_MEDIA_ADDR=127.0.0.1:<port>` for the native media health service.
- Stale or broken local schema: run `scripts/frame reset`. This is intentionally
  unavailable for remote resources.
- Worker bundle failure: install `worker-build` at the version used by CI and
  rerun the Wrangler dry-run command from `CONTRIBUTING.md`.
- macOS/Windows/Linux capture, permission, camera, audio, and hardware-codec
  evidence belongs to the representative-device lanes; a local synthetic
  smoke does not claim those gates.

`dev/local.env.example` contains only safe local defaults and is never sourced
implicitly. `scripts/frame` refuses local mutation whenever production/provider
credentials are present.
