# Upstream Cap reference

Frame uses a shallow, ignored checkout of `https://github.com/CapSoftware/Cap.git` in `.tmp/cap` as a parity oracle. The scaffold was researched against commit `6ba69561ac86b8efdb17616d6727f9638015546b` from 2026-07-15.

Recreate the checkout with:

```sh
mkdir -p .tmp
git clone --depth 1 https://github.com/CapSoftware/Cap.git .tmp/cap
```

At the pinned snapshot, Cap is already a mixed Rust and TypeScript monorepo rather than a JavaScript-only application. It contains a Tauri 2 desktop app and extensive native Rust media/capture crates, plus a Next/React web app, a TypeScript web backend, MySQL/Drizzle persistence, S3-compatible and Google Drive storage, and non-Rust media-service orchestration.

The migration strategy must therefore inventory, preserve, or adapt useful Rust behavior instead of rewriting it for its own sake. The target work is replacement and consolidation of runtime boundaries, the remaining TypeScript control plane and UI, the MySQL-to-D1 data model, and FFmpeg-oriented paths where GStreamer is deliberately selected.

## Upstream Rust disposition

This is a responsibility-level disposition, not permission to copy code. The
pinned inventory and per-file license review remain the authority for any later
adaptation.

| Upstream Rust responsibility | Frame disposition | Evidence owner |
|---|---|---|
| Tauri lifecycle and desktop command concepts | evaluate individually; replace the public IPC with Frame's typed capability surface | issue 33 |
| screen/window capture adapters | behavior retained; replace or independently adapt only after per-file provenance review | issues 24 and 33 |
| audio/camera capture and synchronization | retain behavior behind provider-neutral ports; evaluate implementation individually | issue 25 |
| recording/project formats | read-only compatibility parser plus versioned Frame format; never mutate originals | issues 26–27 |
| native transforms and FFmpeg-oriented orchestration | replace with audited GStreamer graphs except an explicitly retained reviewed fallback | issues 22–29 |
| update, hotkey, tray and platform utilities | evaluate individually against capability, signing and accessibility requirements | issue 33 |

No upstream crate is retained wholesale by default. A change that adapts one
records its exact path, commit, license, modifications, notices, tests, and
rollback owner.

## Provenance

Cap uses mixed licensing: much of the repository is AGPL-3.0 while some capture crates have permissive licenses. This repository does not vendor the checkout or copy source from it. Before adapting implementation code, record the source path, commit, license, and resulting obligations in the implementing pull request.
