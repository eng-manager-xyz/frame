# Local authentication evidence

This record describes locally reproducible evidence for the runtime-neutral authentication core.
It is not production D1, email/SMS delivery, provider, browser-matrix, or legacy-session cutover
evidence.

## Commands

```sh
cargo fmt --all -- --check
cargo test --locked -p frame-domain -p frame-ports -p frame-application
cargo clippy --locked -p frame-domain -p frame-ports -p frame-application --all-targets -- -D warnings
cargo check --locked --target wasm32-unknown-unknown -p frame-domain -p frame-ports -p frame-application
cargo clippy --locked --target wasm32-unknown-unknown -p frame-domain -p frame-ports -p frame-application -- -D warnings
python3 scripts/ci/check-secrets.py
```

The reviewed implementation produced 43 application tests, 26 domain tests, and 23 ports tests
(92 total), plus passing native/Wasm strict lint and formatting gates. The repository secret scan
also passes with the auth files included in an isolated index.

## Proven behaviors

| Area | Local proof |
| --- | --- |
| Signup | Verified unowned identifier mints a one-time provisioning grant; owned target is a suppressed decoy; new identity has no tenant grants |
| Provisioning races | Random-ID forgery denies; concurrent cloned grant has one winner; replay denies; two valid grants racing for one identifier have one winner |
| Verification | Known/unknown issue work is fixed-shape; expiry, attempts, replay, key rotation, and suppression are covered |
| Sessions | Issue/authenticate/rotate/replay/revoke/logout-all/version mismatch and hash-key migration are covered |
| Browser boundary | Exact origin, Fetch Metadata, CSRF cookie/header match, and one-use repository mutation grants are enforced |
| Recovery | Existing sessions and session-bound OTP/OAuth link continuations are revoked/purged before a fresh capability can issue |
| Account linking | Unowned targets can link; owned/self/cross-user targets deny or suppress; logout/logout-all/recovery invalidate pending links; completion returns no login grant |
| OAuth | State, S256 PKCE, callback, audience, preflight ordering, replay, post-provider expiry, provider failure audit, and Google-style authorization-code grammar are covered; the provider-free begin/preflight/finalize repository lifecycle also passes compiled local Worker/D1 conformance without persisting raw provider codes |
| API keys | Tenant role/scope/expiry/revocation and hash-key overlap are enforced in repository conformance; signup cannot self-grant authority |
| Abuse controls | Identifier/source/device/global limits work independently; fallback histories merge; global denial short-circuits new cardinality; TTL and hard cap are tested |
| Delivery outbox | Lease reclaim, stale acknowledgement fencing, retry exhaustion, suppressed cleanup, and redaction are covered |
| Audit | Transaction rollback includes audit state; expiry/provider failures use stable reasons; adapter details remain redacted |

## Independent review findings resolved

The adversarial review found and verified fixes for:

- stale preflight time reused after OAuth provider I/O;
- OTP/OAuth account-link continuations surviving session revocation;
- public unauthenticated identity/provider provisioning mutations;
- unbounded attacker-controlled rate-bucket allocation;
- self-owned account-link delivery and login-capability minting;
- missing expiry/provider-failure audit distinctions;
- missing typed PKCE S256 authorization data;
- absent provisioning forgery/replay/race regressions; and
- an authorization-code parser incompatible with Google-style slash-bearing or short codes.

## Evidence boundary and remaining gates

Before issue 13 or production authentication can close, the project still needs:

- remote D1 contention/replication and provider-induced delayed-commit fault injection beyond the passing compiled local Worker/D1 repository suite;
- production CSPRNG, authenticated-encryption delivery sealer, and provider/email adapters;
- the approved legacy session compatibility or forced-login rehearsal;
- cross-client browser/desktop/mobile/extension fixtures through real transports;
- protected provider callback, email delivery/retry, key rotation, and revocation evidence; and
- operator/security approval attached to the immutable release record.

Absent protected evidence is a promotion blocker and is not represented as a local pass.
