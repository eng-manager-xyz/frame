# Upstream Cap reference

Frame uses a shallow, ignored checkout of `https://github.com/CapSoftware/Cap.git` in `.tmp/cap` as a parity oracle. The scaffold was researched against commit `6ba69561ac86b8efdb17616d6727f9638015546b` from 2026-07-15.

Recreate the checkout with:

```sh
mkdir -p .tmp
git clone --depth 1 https://github.com/CapSoftware/Cap.git .tmp/cap
```

At the pinned snapshot, Cap is already a mixed Rust and TypeScript monorepo rather than a JavaScript-only application. It contains a Tauri 2 desktop app and extensive native Rust media/capture crates, plus a Next/React web app, a TypeScript web backend, MySQL/Drizzle persistence, S3-compatible and Google Drive storage, and non-Rust media-service orchestration.

The migration strategy must therefore inventory, preserve, or adapt useful Rust behavior instead of rewriting it for its own sake. The target work is replacement and consolidation of runtime boundaries, the remaining TypeScript control plane and UI, the MySQL-to-D1 data model, and FFmpeg-oriented paths where GStreamer is deliberately selected.

## Provenance

Cap uses mixed licensing: much of the repository is AGPL-3.0 while some capture crates have permissive licenses. This repository does not vendor the checkout or copy source from it. Before adapting implementation code, record the source path, commit, license, and resulting obligations in the implementing pull request.
