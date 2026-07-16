# Cross-repository preview v1 fixtures

This directory pins the inputs and HTTP policy for the credential-free local
portfolio-to-Frame harness. It reuses the canonical public JSON fixtures under
`fixtures/frame-api/v1` and the generated, non-decodable 127-byte payload under
`fixtures/hermetic/v1`.

The evidence class is `local_semantic_fake`. These files contain no production
response, private recording, account data, cookie, token, signed URL, provider
identifier, or real browser trace. They do not assert compatibility with
Cloudflare, Render, R2, Media Transformations, an upstream EngManager build, or
any browser engine.

Do not replace the synthetic payload or rewrite v1 in place after a release
gate consumes it. Add a new version and preserve the old contract and digest.
