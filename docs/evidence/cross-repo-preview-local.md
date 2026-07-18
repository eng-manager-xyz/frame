# Cross-repository local preview evidence

Frame's credential-free cross-repository preview gate is:

```sh
python3 -I scripts/ci/cross-repo-preview-e2e.py --self-test --timeout 20
```

The command is the locally reproducible evidence layer for the parts of issues
37 and 43 that do not require the EngManager repository, a browser engine, or
provider infrastructure. It uses only Python's standard library, checked-in
synthetic fixtures, temporary files, and loopback HTTP. It refuses to start
when production/provider configuration is present.

Policy and mutation gates are separate and credential-free:

```sh
python3 -I scripts/ci/check-cross-repo-contract.py
python3 -I scripts/ci/test-cross-repo-contract.py
cargo test --locked -p frame-client seeded_producer_changes_match_the_current_consumer_contract
```

They prove the canonical digest inventory, all five producer/consumer lane
definitions, one additive acceptance, five breaking rejections, and nine
unsafe policy mutations, including static-patch promotion. They do not execute
the protected external consumer.

Local execution on 2026-07-16 passed the policy checker (five lanes and six
cases), all nine policy mutations, the 22-test core and 23-test all-feature
`frame-client` suites, one full two-origin journey, and all six named negative
controls. The restart exercise closed the original listener before rebinding
its exact origin and proved recovery; no retry replaced a failed attempt.

## Topology and authority boundary

The orchestrator starts two independent child processes:

```text
browser-like client
  ├─ http://localhost:<ephemeral>  portfolio semantic fake
  │    └─ bounded anonymous background GET only
  └─ http://127.0.0.1:<ephemeral> Frame semantic fake
```

The different hostnames and ephemeral ports are distinct origins. The
portfolio page contains an ordinary absolute top-level link with no query,
fragment, userinfo, bearer value, or return state. A cookie jar visits the
portfolio, follows that link, establishes an independent Frame session, and
returns to the portfolio. Redacted JSONL audits retain cookie *names* and
authorization-presence booleans only, proving that each process sees only its
own host-local cookie and that the background public-health poll has no cookie
or authorization dependency. Cookie values and response bodies are never
written to an artifact.

The portfolio background poll has a 120 ms operation deadline, a 64 KiB body
limit, deterministic bounded polling intervals, cancellation with process
shutdown, an exact v1 major-version check, and last-good state. The request
handler reads only that in-memory state. Normal, malformed, incompatible,
`503`, slow, and stopped-Frame cases all retain the static link and portfolio
content within the 180 ms local handler budget.

## Contract evidence

The harness verifies the following against
[`contract.json`](../../fixtures/cross-repo-preview/v1/contract.json):

| Boundary | Local evidence |
|---|---|
| `frame-client` public JSON | Health, public share, and safe error responses are byte-for-byte the canonical v1 fixtures with exact content type and response status. |
| Dynamic/API cache policy | Every health, share, error, auth, media, caption, and privacy response has the exact no-store/security header set, `Vary: Origin`, no CORS opt-in, and no cache-hit marker. |
| Public media | Full `HEAD`, bounded and suffix `Range`, `416`, content length/type, `Content-Range`, ETag, and synthetic WebVTT captions are checked. |
| Privacy | Private, deleted, failed, unavailable, and missing shares are byte-identical. A public-to-private transition immediately returns the generic body and revokes media. |
| Cache isolation | A private/auth response cannot be public or HIT. A fingerprinted synthetic asset deterministically transitions from semantic MISS to HIT without a cookie. |
| Portfolio availability | Static routes and the accessible Frame link remain usable during malformed, incompatible, error, slow, and complete Frame outage cases. Rollback, retry, same-origin restart, and reconnect restore the background snapshot. No upstream body or private marker enters portfolio HTML. |
| Auth boundary | Anonymous landing/login render, dashboard denial, host-local entry, logout revocation, and repeat denial remain entirely on the Frame origin. |
| Cleanup | Every path, including expected negative-control failure, terminates both process groups and proves both listeners are closed before its temporary directory disappears. |

Six deliberate defects prove the assertions are live: shared cookie domain,
private cache hit, Range off-by-one, unavailable-title disclosure, and a
handler-path upstream fetch, plus a sensitive audit field. Each control must
fail at its named invariant;
an unrelated failure does not count. The quality workflow runs the positive
journey and all controls without retries, so the first unexpected failure
remains release-blocking.

## Evidence boundary

This is `local_semantic_fake` evidence. It intentionally does **not** close or
replace the following protected work:

- a commit and pull request in the independently owned EngManager repository;
- the pinned consumer build under EngManager's own nightly toolchain and
  lockfile;
- a real browser's navigation history, service worker, keyboard, screen
  reader, reduced-motion, CSP, CORS, cookie, and media behavior;
- Render preview creation, `noindex`, expiration, secret isolation, and cleanup;
- Cloudflare Worker/D1/R2 routing, cache traces, Media Transformations, DNS,
  canonical-domain, cost, or rollback evidence; or
- a protected provider canary and paired cross-repository deployment record.

Those records remain required in the protected evidence class described by
[Verification evidence](README.md). A local pass cannot be promoted or
renamed into provider, browser, upstream-consumer, or production evidence.
