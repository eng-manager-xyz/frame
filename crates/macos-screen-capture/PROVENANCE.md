# Provenance and licenses

## Cap reference checkout

The mechanical-migration reference was the ignored checkout at `.tmp/cap`,
pinned to Cap commit:

`6ba69561ac86b8efdb17616d6727f9638015546b`

The following MIT-family files were inspected as behavioral references. Their
SHA-256 values make the reviewed source set reproducible:

| Reference file | SHA-256 |
| --- | --- |
| `crates/scap-screencapturekit/src/capture.rs` | `31c014be83c88476cd7ef9925eff2fbc9b345a207975047ffa23f13ba4d6fec3` |
| `crates/scap-screencapturekit/src/config.rs` | `c2bbe4aae948a4808b9feae950c56d158ad285dc353ddae157095f5736d30b84` |
| `crates/scap-screencapturekit/src/permission.rs` | `d391733515214697a383b244053badecc8e418e92909263ae92da5492594b5ab` |
| `crates/scap-screencapturekit/src/lib.rs` | `9a6c69cb87b761ce49c2e88990b0f040543aae082c0f24786c9e018484df027f` |
| `crates/scap-targets/src/platform/macos.rs` | `1e98ccd08852fb304fd46733fdbe065b6832ac1dfce2952bd52c27d8989b4477` |
| `crates/scap-ffmpeg/src/screencapturekit.rs` | `2f67eaffaaee7f74dc44d6f74e213f464c717689640d603fd2bb874dd2a67d61` |

Cap's root `LICENSE` says code in the `scap-*` family is MIT-licensed. The
referenced MIT text is `licenses/LICENSE-MIT`, copyright 2023 Cap Software,
Inc., with SHA-256:

`8078cb8be22f999ac72697dc96d930031203d18a52f22edd8f20b0f9f4defc1a`

No Cap source text, AGPL recording implementation, private patch, or git
dependency is copied into this crate. The implementation uses Frame's native
contracts and the safe APIs of published registry crates. The reference is
retained only to audit migration coverage until the repository's migration
closure review removes `.tmp`.

## Direct native dependencies

| Crate | Exact version | Registry license | Upstream |
| --- | --- | --- | --- |
| `screencapturekit` | `8.0.0` | MIT OR Apache-2.0 | <https://github.com/doom-fish/screencapturekit-rs> |
| `core-graphics` | `0.25.0` | MIT OR Apache-2.0 | <https://github.com/servo/core-foundation-rs> |

Both versions are exact workspace pins and resolve through crates.io. Cargo.lock
records the complete transitive graph.
