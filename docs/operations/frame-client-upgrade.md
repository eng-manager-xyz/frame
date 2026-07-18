# Upgrade or roll back a pinned `frame-client`

This procedure is for the EngManager portfolio consumer. It deliberately uses
an exact Frame commit and does not add a request-time dependency to a page
handler.

## Upgrade

1. Select a reviewed Frame commit whose `frame-client` matrix, fixture checker,
   Worker contract tests, and privacy review passed. Record its full
   40-character SHA.
2. In the consumer workspace, pin the dependency with `rev = "<full-sha>"`.
   Keep `default-features = false`; enable `client` only in the background
   refresh component that actually performs HTTP.
3. Copy or consume the matching `fixtures/frame-api/v1` artifacts and run the
   consumer fixture suite against every public, processing, unavailable,
   malicious, and forward-additive case.
4. Run the consumer's complete workspace tests and build its production
   artifact. Commit `Cargo.toml` and the root `Cargo.lock` together. Verify the
   lock entry resolves to the same full Frame SHA and that no second
   `frame-client` source exists.
5. Refresh Frame data outside request handlers using a short deadline. Publish
   an immutable last-known-good snapshot only after DTO validation succeeds;
   on timeout, privacy failure, incompatible major, or malformed input, retain
   the prior snapshot or render a static Frame link.
6. Exercise the credential-free two-origin preview: portfolio HTML stays on
   the apex, Frame links stay on `frame.engmanager.xyz`, cookies never cross,
   private fixture markers never render, and Frame failure does not fail the
   portfolio route.

Example dependency shape (replace the placeholder before committing):

```toml
frame-client = { git = "https://github.com/eng-manager-xyz/frame", rev = "<40-character-frame-sha>", default-features = false, features = ["client"] }
```

## Deprecation and major upgrades

Read `api_version.major` before publishing a snapshot. Additive fields and
unknown capabilities need no pin change. For a new major, land dual-version
consumer fixture coverage first, move the pin during the documented N/N-1
window, and remove old-major support only after the portfolio production lock
and rollback revision both support the new contract.

## Rollback

Restore the preceding reviewed dependency declaration and its matching root
lockfile as one change, rebuild, and deploy the immutable prior consumer
artifact. Do not hand-edit a lockfile SHA and do not compensate by weakening
DTO validation. Frame keeps N-1 available during the compatibility window, so
the rollback must not require an emergency server mutation.

Record the old/new Frame SHAs, fixture/schema digest, consumer build result,
preview evidence, deploy identifier, and rollback decision in the release
record. A real consumer checkout, lockfile mutation, or deployment requires
authorization in that repository and cannot be substituted by Frame-local
evidence.
