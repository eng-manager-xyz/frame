# Frame public API v1 fixtures

These synthetic files are the canonical consumer contract for the anonymous
Frame API. Additive fields are compatible within v1; a semantic or type change
requires a new major version and parallel support through the deprecation
window.

`share.private.json`, `share.deleted.json`, `share.failed.json`, and
`share.unavailable.json` are intentionally byte-for-byte identical. An
anonymous consumer cannot distinguish those states or receive a title,
thumbnail, owner/tenant identifier, internal object key, signed URL, comment,
transcript, or session data.

Playback and caption descriptors contain same-origin API paths only. They are
not R2 object names or bearer URLs. Fixtures must remain synthetic and must not
contain production responses, cookies, tokens, personal data, or real media.
