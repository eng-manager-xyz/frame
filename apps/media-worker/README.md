# Frame media worker

The native worker keeps database and object-storage credentials out of the media process. It
claims tenant-scoped jobs from the control plane, downloads and verifies a bounded source,
renders `thumbnail_v1` with GStreamer, uploads the bounded PNG, and completes the lease through
the private worker protocol.

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
development), where each decode runs in a child process with enforced CPU, file-size, resident
memory, and wall-clock ceilings. The binary's diagnostic and health-only commands remain
portable, but a configured consumer on Windows or a Unix host without the required `/bin/sh` and
`/bin/ps` sandbox primitives reports not-ready and exits before claiming work. RSS measurement
failure also terminates the child; it never disables the memory ceiling.

Lease and bearer tokens, private object keys, response bodies, and scratch paths are never
logged. Returned transport paths are accepted only when they exactly match the claimed job's
private same-origin endpoints. Scratch directories are per-attempt, private on Unix, and removed
on every terminal pipeline path.
