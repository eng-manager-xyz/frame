# Frame media worker

The native worker keeps database and object-storage credentials out of the media process. It
accepts the versioned 14-profile native catalog, streams one or more private sources into a
per-attempt scratch directory, executes an audited GStreamer child plan, streams one immutable
output back through the control plane, and completes the fenced lease. The worker continues to
read the legacy singular thumbnail claim during rollout; new claims use dense `sources` and
`outputs` descriptors.

Four graphs are implemented locally today: PNG `thumbnail_v1`, canonical JSON `probe_v1`,
`audio_presence_v1`, and bounded JSON `waveform_v1`. Every remaining catalog row has a stable
graph recipe, pinned factory set, and typed exception, and fails closed with `unsupported_media`;
it is not advertised as executable. In particular,
setting `FRAME_NATIVE_H264_AAC_APPROVED=approved-v1` does not by itself enable an H.264/AAC
output. The relevant trusted plugins, legal/product approval, and an implemented graph are all
required. See [NATIVE_EXECUTOR_V1.md](./NATIVE_EXECUTOR_V1.md) for the exact matrix and exception
list.

For a single deterministic local attempt (including the no-work result), run:

```sh
FRAME_MEDIA_WORKER_ENV=local \
FRAME_CONTROL_PLANE_ORIGIN=http://127.0.0.1:8787 \
FRAME_MEDIA_TENANT_ID=018f47a6-7b1c-7f55-8f39-8f8a86900102 \
FRAME_MEDIA_WORKER_TOKEN='<service API token>' \
cargo run -p frame-media-worker -- work-once
```

The API token must belong to an active tenant member and carry the exact `frame:worker` scope.
No Cloudflare provider credential is used by this process. The local origin must be loopback;
production mode requires HTTPS. `serve` continues to expose `/health/live` and `/health/ready`.
When all three protocol variables are present it also runs the bounded polling consumer; when
they are all absent it remains health-only. Partial protocol configuration fails closed.

The job consumer is supported on Unix hosts (Linux in production and macOS for local
development), where each plan runs in a child process with catalog-derived CPU, file-size,
resident-memory, scratch-disk, output, and wall-clock ceilings. The binary's diagnostic and
health-only commands remain portable, but a configured consumer on Windows or a Unix host without
the required `/bin/sh` and `/bin/ps` sandbox primitives reports not-ready and exits before
claiming work. A failed resource measurement kills and reaps the child; it never disables the
ceiling.

Lease and bearer tokens, private object keys, response bodies, checksums, and scratch paths are
never logged. Returned transport paths are accepted only when they exactly match the claimed
job's private same-origin endpoints. Source bytes and output bytes are incrementally hashed and
never accumulated into an in-memory media-sized buffer. Scratch directories are private on Unix
and removed on success, failure, timeout, cancellation, heartbeat loss, and transport failure.
