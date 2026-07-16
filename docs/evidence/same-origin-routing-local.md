# Issue 39 same-origin routing local evidence

Status: repository contract complete; protected provider evidence pending.

## Locally proved

- The production Render Blueprint declares exactly one custom domain,
  `frame.engmanager.xyz`, and the production API origin remains same-origin.
- Wrangler declares one query-safe route,
  `frame.engmanager.xyz/api*`, for `engmanager.xyz`; production
  `workers.dev` is disabled and the Worker validates the exact HTTPS Host.
- The raw control-plane classifier owns only `/api` and `/api/…`, rejects
  percent encodings, semicolons, backslashes, repeated slashes, and dot
  segments inside that boundary, and maps broad-route lookalikes to a
  non-cacheable 404.
- The checked-in matrix covers 26 route-owner cases, 8 host cases, and 8
  transport classes. It includes query strings, trailing/repeated slashes,
  literal/encoded dot segments, semicolons, encoded separators, `/apix`,
  `/apiary`, uppercase, and an encoded prefix.
- Production response policy supplies one normalized request ID, stable JSON
  errors, no-store headers, safe method handling, fixed-length request streams,
  R2 response streams, and verified single-range semantics. No API upgrade
  surface or gateway request reconstruction exists.
- The ownership inventory names one owner for production and staging DNS,
  Worker route/script, Render service/domain, certificates, and shared zone
  phases. Staging remains explicitly pending; no preview URL is mislabeled as
  provisioned staging.
- The DNS-only, CAA/TLS, Full (strict), default-hostname, cache, monitoring,
  Worker Route rollback, CNAME rollback, and unrelated-host non-regression
  procedures are executable and independently reversible.
- The read-only live runner passed its loopback fake-edge self-test across 18
  Worker/edge route cases, six methods, chunked body handling, metadata spoof
  rejection, upgrade rejection, and `206`/`416` range behavior. This validates
  the runner, not Cloudflare or Render.

Local commands:

```text
python3 -I scripts/ci/check-same-origin-routing.py --evidence target/evidence/same-origin-routing-local.json
python3 -I scripts/ci/same-origin-live-conformance.py --self-test --evidence target/evidence/same-origin-live-runner-self-test.json
cargo test --locked -p frame-control-plane --test same_origin_routing_v1
ruby scripts/ci/check-render-blueprint.rb
python3 -I scripts/ci/check-cloudflare-zone-contract.py
```

The generated JSON report is status-only and states that provider state was
not changed. This local lane uses no Cloudflare, Render, or DNS credentials.

## URL normalization limitation

Local parsing proves how the Worker classifies an observed raw target; it
cannot prove what the configured Cloudflare zone presents after URL
normalization. Cloudflare may apply baseline or configured normalization
before Workers. The protected lane must compare raw and normalized traces,
including `/%61pi`, encoded slashes, dot segments, backslashes, and repeated
slashes. If a lookalike becomes an API pathname before the Worker sees it, the
launch stays closed until the authoritative zone owner adds a host-scoped
`raw.http.request.uri.path` guard or an approved gateway owns normalization.

## Protected evidence still required

No provider state was changed and no production assertion is made. The
following evidence requires protected Render/Cloudflare/portfolio access:

- authoritative DNS history and semantic no-op zone plan;
- exact Render custom-domain verification and origin certificate;
- CAA eligibility, edge certificate, Full (strict), and renewal trace;
- raw and normalized exhaustive route-owner results plus single-hop request
  IDs;
- API/auth/health cache bypass and issue 41 hashed-asset HIT evidence;
- disabled `*.onrender.com` bypass result;
- Worker Route removal, DNS-only, and Render default-hostname rollback
  rehearsals with timestamps;
- unchanged apex, `www`, shop/store, Stripe webhook, and portfolio cache
  behavior.

These are protected launch blockers, not skipped local tests.
