# Build and test the macOS display recorder locally

The `macos-native` release composition is a real, narrow production path. It
can capture one full display, preserve unchanged-screen time, write a VP8/WebM
recording, optionally mux exact 48 kHz stereo system audio as Opus, seal it,
and export a byte-identical Editable WebM. It is not the complete Studio
product described by issue 27: microphone, camera, window/region capture,
pause/resume, project journaling/recovery,
timeline edits, edit-aware rendering, MP4 distribution output, distribution
signing, and notarization remain separate deliverables.

## Build the exact local application

Prerequisites are macOS 13.0 or newer, Xcode command-line tools, the repository
Rust toolchain, Trunk, Tauri CLI 2, and the GStreamer installation resolved by
`pkg-config`. A stable Apple Development or Developer ID code-signing identity
is required for a reliable ScreenCaptureKit test on current macOS releases.
List the identities installed in your login keychain:

```sh
cargo install tauri-cli --version "^2.0.0" --locked
scripts/frame desktop-macos-identities
```

Set the exact identity reported by that command, then build the local release
bundle. The build command signs the whole application, binds its plist and
resources, verifies the signature, and requires the signed identifier to be
`xyz.engmanager.frame`:

```sh
export FRAME_CODESIGN_IDENTITY='Apple Development: Your Name (TEAMID)'
scripts/frame desktop-macos-bundle
scripts/frame desktop-macos-open
```

Do not launch `target/release/frame-desktop` for the physical test. Its linker
identity is not the application bundle identity macOS TCC uses. Always launch
the verified `.app` through `scripts/frame desktop-macos-open`. After granting
Screen & System Audio Recording, quit Frame and run that same open command
again without rebuilding; the grant does not take effect in the requesting
process.

When `FRAME_CODESIGN_IDENTITY` is unset, the bundle command uses the same
identifier-stable ad-hoc fallback as Screen. This makes the bundle internally
consistent and is useful for non-capture shell testing, but macOS 15 and newer
can still reject it for ScreenCaptureKit. It is not proof that physical capture
works.

Do not move this local development application out of the checkout's
canonical `target` directory. It deliberately borrows only the audited
build-machine GStreamer runtime and is not a distributable release.

Changing from the old linker/ad-hoc identity to the stable signed bundle
requirement invalidates the old privacy row once. With Frame quit, explicitly
reset only that row, then launch, approve the prompt, quit, and reopen:

```sh
tccutil reset ScreenCapture xyz.engmanager.frame
scripts/frame desktop-macos-open
# Approve the macOS prompt, quit Frame, then:
scripts/frame desktop-macos-open
```

Do not reset `All`; it would discard unrelated Frame privacy choices. Changing
the security setting or approving the system prompt requires the machine
owner's confirmation.

## Record and export

In Frame:

1. Select **Confirm permissions** and accept the macOS prompt.
2. Select **Refresh displays**, then choose one of the opaque Display buttons.
3. Optionally enable system audio in Settings, then select **Start recording**.
4. Leave the display unchanged for at least five seconds, change visible
   content once, then select **Stop**. The status must say the artifact was
   sealed; a failure or a UI that remains Recording is a failed test.
5. Select **Export editable WebM**. The export status must reach Completed.

The sealed original is stored below the app data directory in
`media/recordings`. The exported `Frame-*.webm` is stored below the same app
data directory in `exports`. Keeping the automatic destination app-owned
prevents a protected Downloads-folder prompt from blocking Tauri setup; a
future explicit Save dialog can grant a user-selected destination. Both files
are private `0600` regular files. Captured pixels are written and independently
verified through a preopened descriptor; the final original name appears only
after verification and an identity-checked no-replace rename. Export staging
uses the private `exports/.frame-staging` directory. Frame pins every
participating directory, rejects visible rename/replacement races, and
rehashes the published inode through a rooted descriptor before rebinding the
final filename to the same inode.

The recorder fails closed at four hours, 2,000,000,000 encoded bytes, or less
than 512,000,000 available filesystem bytes. Callback, appsrc, GStreamer, and
stop-tail queues are separately bounded.

When system audio is requested, startup holds at most one video frame and one
audio chunk for at most 80 ms. Their raw epoch-zero ScreenCaptureKit timestamps
establish one shared origin, preserving the real startup offset before either
item reaches GStreamer. Missing comparable video time or no initial audio chunk
fails the A/V session; Frame never fabricates PCM. Disable system audio and
retry to use the verified screen-only fallback. Native permission/start or
required-factory availability failures that prove complete audio teardown also
fall back to screen-only and report that mode in the UI.

## Verify the artifacts

Use the two paths produced by the run:

```sh
gst-discoverer-1.0 /absolute/path/to/original.webm
gst-discoverer-1.0 /absolute/path/to/Frame-export.webm
shasum -a 256 \
  /absolute/path/to/original.webm \
  /absolute/path/to/Frame-export.webm
```

Both probes must report a playable VP8 WebM video; an A/V run must also report
Opus stereo audio at 48 kHz. The duration must cover the five-second unchanged
interval within normal frame-timing tolerance, and the
two SHA-256 values must match because the current Editable WebM export is an
integrity-checked copy of the sealed source.

The source-level and synthetic gates can be rerun with:

```sh
GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked \
  -p frame-macos-screen-capture --all-targets

GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked \
  -p frame-macos-av-capture --all-targets

GST_PLUGIN_SYSTEM_PATH_1_0="$(pkg-config --variable=pluginsdir gstreamer-1.0)" \
  scripts/ci/gstreamer-sanitized-exec cargo test --locked \
  -p frame-media --test screen_recording_contract

python3 scripts/ci/desktop-shell-smoke.py \
  --expected-adapter native_macos_display
```

These automated checks do not substitute for the physical five-second capture
and playback check above.
