# Legacy Cap project import

Frame's native desktop can discover Cap projects without changing the Cap
installation or recording tree. Native macOS can copy supported projects into
Frame Studio. Native Windows currently provides the compatibility report only.

## Safety contract

- Discovery reads Cap's default `recordings` directory and bounded current or
  previous custom recording roots from Cap's `store`/`store.json`.
- Cap projects, settings, and source media are never written, renamed, or
  deleted.
- Symlink projects, path traversal, missing/empty media, source changes, invalid
  JSON, unsupported newer schemas, and exceeded bounds fail closed.
- The WebView receives an ordinal, coarse status/counts, catalog generation, and
  CSPRNG token. Paths, project names, media names, digests, and durable Frame
  identities remain in Rust.
- Import reauthenticates the source, copies and checksums every supported asset,
  commits immutable Frame originals, rechecks the Cap tree, and commits a
  canonical Frame project. A source- and manifest-bound journal receipt keeps
  migration distinct from native recording.

Do not remove the Cap project merely because it is listed as `Imported`.
Retention and legacy-desktop removal remain governed by parity gate 29.

## Local production build

On macOS with the repository prerequisites installed:

```sh
python3 scripts/ci/build-desktop-ui.py
python3 scripts/ci/check-desktop-bundle.py

cargo build --locked --release \
  -p frame-desktop-core \
  --features tauri-app,custom-protocol,macos-native \
  --bin frame-desktop

python3 scripts/ci/desktop-shell-smoke.py \
  --expected-adapter native_macos_display

./target/release/frame-desktop
```

In Settings, choose **Scan Cap projects read-only**. Results deliberately use
ordinals instead of Cap names:

- `Importable`: macOS may copy the project into Frame.
- `Imported`: an authenticated completed Frame journal already binds this Cap
  source.
- `NeedsReview`: unsupported effects or an interrupted attempt require review;
  no new import is started.
- `Unsupported`: the project uses a newer unpinned Cap format.
- `Invalid`: the project is incomplete, unsafe, malformed, or changed.

After a successful import, open the newly refreshed project from the Frame
Studio catalog and verify preview/export behavior before making any retention
decision about the Cap source.

## Recovery

If an import loses only its final acknowledgement after all originals and the
canonical project are durable, the Studio recovery catalog reports
`RecoverLegacyImport`. Recovery reauthenticates Frame's originals and manifest,
checks the prepared journal receipt, and finishes registration without
accessing Cap.

An attempt that failed earlier remains `NeedsReview`. Preserve the Cap source
and Frame data, collect the coarse error/evidence record, and investigate before
retrying or cleaning any partial Frame state.

## Verification

```sh
cargo test --locked -p frame-legacy-import
cargo test --locked -p frame-media --test studio_mode_contract \
  legacy_import_uses_a_distinct_direct_journal_receipt

GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  cargo test --locked -p frame-desktop-core \
  --features tauri-app,macos-native --all-targets

python3 scripts/ci/check-desktop-product.py
```

The checked-in Cap fixture is schema-faithful and synthetic. It proves local
mechanics, source preservation, restart detection, and final-acknowledgement
recovery. It is not the privacy-reviewed historical compatibility corpus
required for release approval.
