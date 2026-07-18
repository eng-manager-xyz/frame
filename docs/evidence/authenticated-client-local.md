# Authenticated first-party client local verification

`frame-authenticated-client` is the provider-neutral wire-contract package for
authenticated first-party Frame applications. It is not the anonymous public
or portfolio contract and contains no HTTP implementation, credential, object
key, provider handle, Worker type, native media type, or GStreamer dependency.

The Instant finalize request binds tenant, session, upload, video, ordered-part
digest, server object version, deterministic job identity/generation, and a
canonical semantic request digest. Retry operation identity is validated but
excluded from that semantic digest so an exact lost-response retry remains
stable.

The receipt contains only state plus publication, request, job, upload,
object-version, and distribution identities. Published and pending states are
validated fail closed. The control plane retains the playable storage identity
internally and the desktop reconstructs its native receipt only after the wire
receipt validates against the exact request.

## Reproducible commands

```text
python3 -I scripts/ci/check-frame-client-contract.py
cargo test --locked -p frame-authenticated-client
cargo clippy --locked -p frame-authenticated-client --all-targets -- -D warnings
cargo check --locked -p frame-authenticated-client
cargo check --locked -p frame-authenticated-client --target wasm32-unknown-unknown
cargo test --locked -p frame-desktop-core --features instant-finalize
cargo clippy --locked -p frame-desktop-core --features instant-finalize --all-targets -- -D warnings
cargo test --locked -p frame-control-plane --lib instant_finalize
scripts/ci/check-workspace-boundaries.sh
```

These checks are credential-free local evidence. They do not prove hosted D1
contention, real R2 object behavior, production bearer/session handling, or a
released desktop build.
