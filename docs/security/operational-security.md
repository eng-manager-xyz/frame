# Operational security closure and response

The canonical threat boundaries remain in `docs/security/threat-model.md`.
Issue-34 closes local controls through pinned workflows, secret scanning,
Cargo advisory/license/source gates, CycloneDX 1.6, deterministic provenance,
privacy-safe diagnostics, recovery manifests, and fail-closed games. An
independent penetration test and deployed review remain protected evidence.

## Vulnerability handling

Run all four `cargo deny` gates, the secret scanner, SBOM generator, native
runtime inventory, and normal tests on the exact release SHA. A database outage
fails the advisory gate. Triage by affected shipped surface and reachable
feature, preserve the first result, assign owner/severity/deadline, patch and
rebuild every affected subject, rotate if exposure is plausible, and attach a
new signed provenance/SBOM. No critical/high finding is allowed at promotion.
Exceptions must be exact, reasoned, owned, dated, expiring, and independently
reviewed; privacy or cross-tenant findings cannot be waived.

## Rotation rehearsal

Rehearse Cloudflare deployment, R2 signing, session hash, webhook HMAC, desktop
update, and backup-recovery credential classes at their policy cadence. Create
the least-privileged replacement, verify new issuance/effects use it, retain
only the bounded overlap required for in-flight credentials, revoke the old
authority, probe rejection, inspect audit/replay state, and record key IDs and
receipt digests only. Never print or attach key material. A local metadata-only
test cannot claim provider revocation or custody recovery.

## Penetration and contacts

The independent scope includes public web, API, auth, upload, share, provider
callbacks, and desktop update. Findings bind the tested release and environment
and must include retest disposition. Personal contacts live in the protected
registry; the repository stores roles only. Exercise incident commander,
release, security, recovery, Media, and support acknowledgements. Missing or
stale contact/penetration records block the applicable promotion.

## Diagnostic privacy

Only the support-bundle allowlist may leave an operator workstation. Reject
unknown fields rather than redacting after collection. Do not collect media,
captions, bodies, provider messages, signed URLs, tokens, raw email, private
titles, object keys, tenant/user IDs, or paths. Treat a diagnostic leak as a
security incident: stop distribution, revoke exposed capability, remove access,
inspect downloads, and retain a privacy-safe timeline.
