# Frame public API v1 fixtures

These synthetic files are the canonical consumer contract for the anonymous
Frame API. Additive fields are compatible within v1; a semantic or type change
requires a new major version and parallel support through the deprecation
window.

`contract.schema.json` is the reviewed JSON Schema 2020-12 artifact. Its
definitions intentionally allow unknown object fields for N/N-1 additive
compatibility while constraining all known privacy-sensitive paths and values.
The Rust DTO validators remain authoritative for same-origin checks and exact
runtime capability paths that JSON Schema cannot bind to a configured origin.

`share.private.json`, `share.deleted.json`, `share.failed.json`, and
`share.unavailable.json` are intentionally byte-for-byte identical. An
anonymous consumer cannot distinguish those states or receive a title,
thumbnail, owner/tenant identifier, internal object key, signed URL, comment,
transcript, or session data.

Playback and caption descriptors contain same-origin API paths only. They are
not R2 object names or bearer URLs. Fixtures must remain synthetic and must not
contain production responses, cookies, tokens, personal data, or real media.
