# Local synthetic media-runtime evidence

This record covers only hardware-free synthetic work from issues 22, 23,
25 and 29. It does not claim physical capture devices, OS permission prompts,
hardware encoders, clean-machine installers, signing/notarization, Cloudflare
Media, or long-duration platform soaks.

On 2026-07-16, the native media suite passed with Frame Media 0.1.0 and
GStreamer/plugins 1.28.0 on macOS:

- 59 media and 19 worker unit/runtime tests, including a real 320×180 VP8 plus
  48 kHz Opus WebM;
- positive `V_VP8` and `A_OPUS` track/caps checks plus pre-encoder
  source-timeline startup/drift probes; decoded artifact-level per-track sync
  remains pending;
- cancellation, deadline, blocked-sink, startup-error, streaming-error and
  terminal-once cases, all with an attempted and reported `Null` teardown;
- dedicated owner-thread pause/resume/finish commands and transition/terminal
  events over bounded channels, plus rejection of an actual unbounded queue and
  a trusted-root but undeclared authored factory;
- rejection of plugin and native-loader overrides before readiness discovery,
  with a Cargo runner that clears loader search variables at the final Unix
  executable boundary;
- privacy tests proving a private missing-file path is absent from the complete
  structured report;
- 12 post-warmup start/stop cycles, 12 completed artifacts, maximum observed
  A/V drift 5,333,333 ns, and no lifecycle gate failures;
- strict all-target media clippy with warnings denied.

The deadline tests prove bounded polling/state-confirmation and cooperative
behavior for the audited synthetic plugins. Inline plugin callbacks are not
preemptible, so a hard wall-clock watchdog/process-isolation claim remains
pending. Execution and `Null` teardown elapsed times are reported separately.

The local desktop sandbox denied non-privileged `ps` process inspection, so the
local JSON correctly records RSS/thread/handle samples as unavailable rather
than fabricating them. The Ubuntu CI lane reads the same process's `/proc`
status and file-descriptor directory and gates start/inter-cycle-peak/end
trends, failing closed if those measurements are unavailable. Protected
platform soak evidence remains pending.

Reproduce using the commands in
`docs/operations/gstreamer-runtime.md`. Generated JSON and WebM files remain in
ignored `target/evidence`; the repository contains only this bounded summary.
