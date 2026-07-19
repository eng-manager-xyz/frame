# GStreamer runtime and pipeline operating contract

Frame treats GStreamer as a native runtime dependency, not as an accidental
developer-tool dependency. The machine-readable source of truth is
`crates/media/gstreamer-runtime.json`; the Rust doctor and CI contract checker
must agree with it exactly.

## Distribution and trust boundary

| Target | Runtime source | Current release gate |
| --- | --- | --- |
| Ubuntu 24.04 x86_64 | Top-level GStreamer packages installed on the GitHub-hosted runner; package name, version and installed status captured | Synthetic CI only; APT origin/signature, version approval, dependency closure and clean-machine gates pending |
| Ubuntu 24.04 aarch64 | Proposed system-package model | Origin/signature, version approval and clean-machine evidence pending |
| macOS x86_64/aarch64 | App-relative, signed and notarized runtime bundle only | Bundle source, signing, clean-machine and license approval pending |
| Windows x86_64 | App-relative, Authenticode-signed runtime bundle only | Bundle source, signing, clean-machine and license approval pending |

The build records the canonical `pkg-config --variable=pluginsdir
gstreamer-1.0` result. Before calling `gst::init`, the launcher must set
`GST_PLUGIN_SYSTEM_PATH_1_0` to that exact build-time root. Setting the
versioned system path suppresses GStreamer's default system search, which
otherwise includes the user-writable XDG plugin directory. Frame rejects
custom plugin paths, scanner paths, registry paths/modes, loading whitelists,
and feature-rank overrides. Diagnostics report only the public variable or
factory name, never an environment value or path.

Native loader overrides (`LD_*` and `DYLD_*` injection/search variables) are
also rejected by public variable name. `scripts/ci/gstreamer-sanitized-exec`
clears them at the final executable boundary, and the Unix Cargo runner invokes
it after Cargo adds its own development search paths. `scripts/frame`, CI and
production preflight enter through this boundary. The launcher must itself be
started by a trusted service manager or CI environment: a shell executable
cannot undo code already injected into its own process by a hostile parent
loader. A signed launch chain, app-relative macOS/Windows loading and Windows
DLL search hardening remain protected issue 22 gates.

The macOS desktop's raw release executable has one additional local-only
bootstrap. Before Tauri or GStreamer creates threads, an executable outside an
application bundle either confirms the exact build-time plugin root or replaces
itself with `GST_PLUGIN_SYSTEM_PATH_1_0` as the sole plugin-search setting. The
replacement uses `exec`, so process identity, signals, exit status, arguments,
and standard streams are preserved; there is no recursion marker that a parent
can forge.
Explicit plugin or loader overrides still fail closed. A locally built
`*.app/Contents/MacOS` executable may use the same path only while its
canonical location remains beneath this checkout's canonical `target`
directory. Canonicalization prevents a parent traversal or symlink from turning
that developer exception into an installed-app fallback. A copied or installed
bundle fails until an audited app-relative runtime, native dependency closure,
signatures, license inventory, and clean-machine evidence are present.

After initialization, every manifest factory's backing plugin filename must
canonicalize under the build-time root. The thumbnail path repeats that audit
over the full live graph after `decodebin` autoplugging, so dynamically selected
decoder elements cannot escape the trusted root. The supervisor rejects a
Frame-authored graph factory absent from the manifest before any state change.
Autoplug children may add factories supplied by the configured package/bundle
root and are audited by provenance after autoplugging. That dynamic surface is
not described as a closed factory-name allowlist.

The Linux CI selection is intentionally narrow: core tools, base, base-apps,
good plugins and build headers. CI rejects differences in that exact top-level
selection and retains package versions plus the doctor/factory inventory. This
is not an APT provenance/signature proof or a recursive native dependency SBOM.
A hostile-XDG regression puts a real plugin copy in a user-writable XDG
directory with a fresh cache and requires every factory provenance check to
remain trusted. A separate hostile-loader regression requires raw worker
readiness to fail closed. macOS and Windows remain release-blocked until the
bundle archive digest, source, app-relative loader paths, signatures,
notarization/Authenticode result, uninstall behavior, and clean-machine A/V
smoke are attached. Local success is not evidence for those protected gates.

## Pipeline behavior

The synthetic gate records a bounded 320×180 VP8 video track and a 48 kHz mono
Opus audio track into WebM. All four queues are explicitly non-dropping and
bounded above the complete finite fixture's buffer count. A single supervisor owns the pipeline,
bus, lifecycle, progress probe, actual queue bounds/levels/overruns, cooperative deadline,
stall watchdog, warning/message limits, state timings, A/V timestamps, and
transition to `Null`. A dedicated owner-thread mode exposes bounded command and
event channels for pause, resume, finish, cancellation, transitions, warnings,
and the single terminal event; one channel slot is reserved for that terminal
notification and the joined report remains the authoritative outcome. Raw
frames never cross that boundary.

Studio asset v2 uses the same shipped codec baseline for originals: VP8/WebM
for each enabled video track and Opus/WebM for each enabled audio track.
`wavenc` remains optional only for the retained two-pass audio-normalization
graph; it is not a production Studio asset encoder. VP9 recording, FLAC
recording, and Matroska recording are not declared as production Studio
factories. A graph that names those older contract-only families is not
migrated into a different codec implicitly.

Operational failures produce one typed terminal outcome. Cancellation,
deadline, blocked-output, startup failure, and streaming-error tests all verify
an attempted teardown for the audited synthetic graph. Polls and state
confirmations are bounded, but inline plugin calls such as `set_state`,
`send_event` and latency recalculation are cooperative and can block if a
plugin misbehaves. This is not a hard wall-clock kill guarantee; external
process isolation/watchdog evidence remains a protected issue 23 gate.
Execution `elapsed_ms` excludes the separately bounded mandatory `Null`
confirmation budget reported by `teardown_elapsed_ms`. Diagnostics contain
only a caller-supplied bounded correlation ID, public application/runtime/plugin
versions, factory/media-type names, counters, durations, and numeric PTS
measurements. They never include buffers, media, environment values, device
labels, local paths, or raw GStreamer error/debug strings.

The A/V PTS probes observe source-timeline buffers before encoding. They gate
startup offset and short-run drift in the synthetic producer graph, not decoded
timestamps from the written artifact. `gst-discoverer` independently verifies
one VP8 track, one Opus track, the container, caps, seekability and global
duration. Artifact-level per-track decoded timestamp/drift and long-duration
sync remain protected issue 25 evidence.

The fast soak warms the plugin registry, then runs at least eight complete A/V
start/stop cycles. Linux evidence records RSS, file-descriptor and thread
start/inter-cycle-peak/end samples and fails closed if `/proc` measurement is
unavailable; other runners record the measurements available without privileged
process inspection. The gate also checks completed-cycle
count, output size, A/V drift, terminal uniqueness, teardown result and
teardown time. This is a fixed-profile synthetic CI regression gate, not the long-run
hardware soak required to close issue 29.

## Codec, license, SBOM, and vulnerability policy

VP8, Opus and WebM are the only codecs/container required by this synthetic
gate. H.264/AAC/MP4 factories remain optional and cannot become a distribution
default until patent, redistribution and product review is recorded. No entry
in the manifest is a legal approval.

The native Cargo dependency inventory, top-level hosted-runner GStreamer
selection, factory manifest, application/runtime/plugin versions, path-free
structured media probe, and contract digest are partial SBOM inputs. They are
not a recursive native-library inventory. The verbose discoverer output is a
temporary CI input and is never uploaded. Release artifacts must add the full
native dependency closure, source/signature evidence, selected macOS or Windows
bundle inventory and license notices.
A GStreamer/plugin CVE is handled through the dependency policy: identify
affected factories, disable the capability or pin the previous approved
bundle, rebuild/sign, rerun synthetic and platform smoke, and retain the old
compatible bundle for rollback.

## Local verification

On a macOS development machine with the audited GStreamer installation used at
build time, the native desktop release binary bootstraps its own trusted plugin
path; do not export a substitute path:

```sh
cargo build --locked --release -p frame-desktop-core \
  --features tauri-app,custom-protocol,macos-native --bin frame-desktop
./target/release/frame-desktop
```

This raw-binary path is suitable for backend boot and GStreamer diagnostics,
but not for a physical ScreenCaptureKit permission test: its linker-generated
code-signing identity is not `xyz.engmanager.frame`. Build, sign, verify, and
launch a release-mode local `.app` with:

```sh
export FRAME_CODESIGN_IDENTITY='Apple Development: Your Name (TEAMID)'
scripts/frame desktop-macos-bundle
scripts/frame desktop-macos-open
```

On current macOS releases, an Apple Development or Developer ID identity is
required for reliable ScreenCaptureKit TCC behavior. The bundle command has an
identifier-stable ad-hoc fallback for shell testing, but that fallback is not
physical-capture evidence.

Do not move that artifact before testing it. This developer exception is not a
distributable bundle and is not clean-machine, signing, notarization, license,
or native dependency-closure evidence for issue 22. Once copied outside the
checkout's canonical `target` tree, it fails rather than borrowing the build
machine's GStreamer installation.

Follow the [macOS display-recording runbook](macos-display-recording-local.md)
for the permission, five-second static-screen, seal, export, duration, and hash
checks. Production display recording uses `fdsink`/`fdsrc` against a retained
private descriptor; path-based `filesink` remains in synthetic and other
non-display media helpers and is not the native capture writer.

```sh
export GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)"
scripts/ci/gstreamer-sanitized-exec cargo test --locked -p frame-media -p frame-media-worker
scripts/ci/gstreamer-sanitized-exec cargo clippy --locked -p frame-media -p frame-media-worker --all-targets -- -D warnings
scripts/ci/gstreamer-sanitized-exec cargo run --locked -q -p frame-media --example runtime_doctor \
  > target/evidence/gstreamer-doctor-local.json
scripts/ci/gstreamer-sanitized-exec python3 -I scripts/ci/check-gstreamer-runtime.py \
  --doctor target/evidence/gstreamer-doctor-local.json \
  --evidence target/evidence/gstreamer-runtime-contract-local.json
scripts/ci/gstreamer-sanitized-exec cargo run --locked -q -p frame-media --example runtime_soak -- \
  --cycles 12 --evidence target/evidence/media-runtime-soak-local.json
```

On Linux x86_64, create the same `dpkg-query` TSV used by the media workflow
and pass it as `--packages`; the checker requires the exact top-level CI
selection but does not claim archive provenance, signatures or transitive
closure. `scripts/frame` derives and exports the trusted plugin root and clears
native loader overrides for `doctor`, `check`, `test`, `start`, and
`media-smoke`.

Evidence must contain synthetic media only. Never upload customer recordings,
device labels, plugin paths, environment values, or raw diagnostic dumps.
