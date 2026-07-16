# Hermetic walking-slice fixture

`v1/manifest.json` binds the journey to the exact bytes in
`v1/synthetic.webm`. Despite the extension and content type, those bytes are
intentionally **not decodable media**. They are generated text with no user,
production, or provider data. The manifest records that provenance and pins
the byte length and SHA-256 digest.

Run the provider-free gate with:

```sh
python3 -I scripts/ci/hermetic-journey.py
```

The gate requires only Python's standard library. It starts a loopback-only
semantic server, uses temporary SQLite and object state, and walks through:

1. actor and tenant rejection;
2. upload intent, CORS, checksum rejection, and direct object upload;
3. finalization and a non-public processing state;
4. an injected managed-media failure followed by the modeled native fallback;
5. public sharing, byte ranges, immutable-asset caching, and no-store media;
6. a public-to-private transition with cache invalidation; and
7. database and object checksum reconciliation.

It then reruns the same journey against a deliberately defective cache model
and requires the privacy leak check to fire. A pass is evidence about these
local semantic contracts only. It is not evidence of Cloudflare, Render,
browser, codec, hardware, network, or production compatibility.
