# Release provenance, signing, promotion, and rollback

Every deployable in
`fixtures/operational-hardening/v1/operational-policy.json` is promoted by
digest, never rebuilt between stages. The release subject includes its full Git
SHA, exact dependency inventory, CycloneDX 1.6 SBOM, artifact SHA-256 values,
migration level, contract major, builder identity, and previous compatible
rollback pointer. Worker, Render web/assets, native media worker, macOS and
Windows desktop channels, and `frame-client` are separate subjects; one passing
subject cannot attest another.

## Reproduce and verify

Build from a clean exact revision with the pinned toolchain, then run the
normal package gate. Generate canonical SLSA v1 provenance:

```sh
python3 -I scripts/ci/release-provenance.py generate \
  --release-manifest target/frame-release/release-manifest.json \
  --output target/frame-release/provenance.json \
  --source-uri https://github.com/eng-manager-xyz/frame \
  --builder-id https://github.com/eng-manager-xyz/frame/.github/workflows/production-gate.yml
```

The production signer uses a protected OIDC identity or offline release key.
The repository never stores the private key or a permissive allowed-signers
file. Verify the exact source, subjects, provenance, and trusted signature:

```sh
python3 -I scripts/ci/release-provenance.py verify \
  --provenance target/frame-release/provenance.json \
  --release-manifest target/frame-release/release-manifest.json \
  --expected-sha "$RELEASE_SHA" \
  --signature target/frame-release/provenance.sshsig \
  --allowed-signers "$FRAME_RELEASE_ALLOWED_SIGNERS"
```

The ephemeral Ed25519 self-test proves signature and tamper semantics only; it
is not a trusted release signature.

## Promotion

Pull-request checks produce untrusted candidates. Staging verifies the same
subject digest, exercises supported N/N-1 schema and client contracts, and
attaches synthetic and restore evidence. Production requires a protected
reviewer, trusted signature, numerical cost approval where provider work is
enabled, current vulnerability results, no critical/high findings, an exact
rollback pointer, and all protected evidence applicable to that subject.
Promotion retains the first failure and cannot waive privacy, cross-tenant
access, corruption, duplicate billing, or acknowledged-write loss.

## Rollback

Worker rollback deploys the previous compatible bundle and leaves expand-only
D1 migrations in place. Render selects the named previous deploy. Media fences
attempts, disables only the affected profile revision, reconciles staging/final
keys, then selects the prior compatible native profile. Desktop channels stop
promotion and republish only a previously signed compatible updater manifest;
clients already updated use a forward fix unless the approved updater contract
supports downgrade. A rollback never rewinds a destructive migration, deletes
source media, overwrites an immutable output, or purges unrelated resources.

Trusted signing, notarization/AuthentiCode, actual promotion, and timed provider
rollback remain protected records in `protected-evidence.json`.
