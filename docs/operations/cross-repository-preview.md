# Pair, verify, and clean cross-repository previews

## Local gate

Run `scripts/frame preview-e2e`. The command refuses production deployment,
provider tokens, or `DATABASE_URL`, uses ephemeral ports/directories, terminates
both process groups on every path, and runs its negative controls without
retries. A failure is release-blocking until assigned or quarantined with an
owner and expiry.

Before pairing anything, run
`python3 -I scripts/ci/check-cross-repo-contract.py` and its mutation suite.
The checker binds this runbook to the exact fixture inventory, producer/
consumer workflow, one-attempt local gate, artifact allowlist, and explicit
`not_collected` protected records. It performs no network or provider action.

## Trusted preview creation

1. Record the full Frame and portfolio SHAs, fixture/schema digest, operator,
   cost budget, expiry no later than three days, and cleanup deadline.
2. Approve the Frame Render preview. Confirm `FRAME_DEPLOYMENT=preview`,
   `IS_PULL_REQUEST=true`, canonical origin from `RENDER_EXTERNAL_URL`, staging
   API only, no production secret, no production cookie, and `noindex`.
3. If a Worker/provider journey is required, create or select named isolated
   staging D1/R2/route resources. Seed only generated public media. Never copy a
   production database, recording, credential, or signed URL.
4. Pair the portfolio preview by exact Frame preview URL/commit. Run top-level
   navigation, degradation, public contract, privacy, routing, header, browser,
   accessibility, and enabled optional-capability matrices.
5. Attach status/header/digest/timing results only. Redact bodies, cookies,
   authorization, queries, object keys, signed URLs, personal data, provider
   identifiers, and media. Retain first-failure artifacts for 14 days and
   successful summaries for the bounded release window.

The trusted profile is fixed at 72 hours, uses the protected
`cross-repository-preview` environment, and records both commits, both preview
identifiers, fixture digest, operator, expiry, and cleanup owner. Production
secrets, data, DNS mutation, and ambient browser authority are prohibited.

## Cleanup monitor

At expiry, delete only the recorded preview service/route/staging namespace,
then prove the Render URL and route are gone, staging objects/rows are empty or
retained by an explicit incident hold, and production DNS is unchanged. A
monitor alerts the owner before and after deadline; it never receives authority
to delete production resources.

## Flakes, timeout, and cost

Fast local gates have one attempt and 20-second orchestration timeout. Browser
and provider lanes may retry once only after preserving the first trace. Each
flake record names the assertion, owner, deadline, seed/browser/provider
version, and whether it blocks release; quarantine cannot turn required
coverage green. Provider canaries use generated sub-megabyte media, bounded
requests, one concurrent run, a monthly budget alert, and an emergency disable.

The protected canary definition caps a run at 100 requests, 1 MiB of generated
media, 20 minutes, concurrency one, and a USD 25 monthly budget. It does not
grant credentials or create a canary: the protected environment, cleanup
record, and kill switch remain prerequisites.

## Rollback

Disable preview pairing or optional status/embed/handoff first, leaving the
static Frame link. Remove staging routes and preview services without touching
production DNS. If a compatibility failure appears, restore the prior portfolio
pin or prior Frame major; never silently copy a fixture or update a lockfile.

Provider URLs, browser/device/assistive-technology reports, preview expiration,
cleanup, consumer builds, and cost traces are protected evidence.
