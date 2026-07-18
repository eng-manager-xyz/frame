# Storage governance operations

## Preconditions

- Keep all origin buckets private and disable unauthenticated provider endpoints.
- Route application, signed, custom-domain, and managed-media reads through the centralized policy.
- Configure exact allowed origins; never use `*` with credentials.
- Configure the privacy-change SLO at or below the charter maximum of 60 seconds.
- Provision separate signing, purge, media, malware, backup, and deletion credentials.
- Verify lifecycle manifests before enabling deletion for a tenant.

## Privacy or delete rehearsal

1. Select synthetic immutable objects covering every `GovernedObjectRole`.
2. Seed a positive response and a 404 on both signed and custom-domain paths.
3. Increment the cache generation and apply the privacy/tombstone state first.
4. Confirm the old signed generation fails at the application immediately.
5. Purge the plan's positive and negative variants.
6. Probe signed, cached, direct-origin, and custom-domain paths until all deny.
7. Record first-denial and final-denial times. Fail if the approved SLO is exceeded.
8. Keep response bodies, URLs, keys, and tokens out of the evidence bundle.

## Legal hold

1. Authenticate an owner/admin or dedicated legal-hold operator.
2. Bind the hold to the tenant and subject digest; store the reason in the restricted audit system.
3. Re-run deletion planning and verify `LegalHoldActive` before any provider mutation.
4. If a hold arrives during deletion, stop at the current durable boundary. Do not compensate by
   guessing keys or restoring from an unverified provider listing.
5. Release the exact hold with a monotonic timestamp. Retry the same manifest-bound deletion plan.

## Restore

Restore only a planned or tombstoned workflow. Verify the same inventory digest, the seven-day local
grace deadline, authorization, and audit chain. Before changing the workflow to restored, require an
exact provider `HEAD` observation for every manifest object: correct bucket, identifier, size, and
checksum. Metadata reactivation and the restored workflow transition are one D1 batch. If any object
is absent (including a provider deletion completed just before a process crash), fail closed. A
workflow at origin-deleted or later requires the separate backup recovery procedure and must not use
the normal restore command.

## Export

Build the plan from the authoritative lifecycle manifest, verify its digest immediately before read,
exclude multipart state, and stream through bounded buffers. A changed manifest invalidates the plan.
Do not use provider listings to add or omit objects. Encrypt the completed export, issue a short-lived
download grant, and delete the export through its own manifest lifecycle.

## Managed-media incident

1. Set `ManagedMediaState::DisabledByIncident` before changing routes or credentials.
2. Stop new provider invocations; leave immutable completed outputs available under normal policy.
3. Reconcile in-flight jobs and quarantine outputs without a valid manifest/checksum.
4. Route eligible work to the declared native fallback. Do not silently relax format or size limits.
5. Rotate exposed credentials, inspect provider audit records, and purge affected cache generations.
6. Re-enable only after synthetic input-origin, isolation, output-integrity, and cost probes pass.

## Quota incident

Freeze new upload grants for the affected tenant, retain multipart cleanup, reconcile reserved versus
committed bytes, and audit discrepancies. Never delete held content to recover quota. A temporary
quota increase requires an owner, expiry, capacity review, and audit event.

## Erasure completion

Require all five evidence digests: tombstone, origin deletion, positive/negative cache purge, backup
deletion/expiry, and verified absence. Generate the privacy-safe proof only after the workflow reaches
`Complete`. Sample provider listings can find anomalies but cannot replace exact manifest evidence.

## Rollout and rollback

Apply policy to new immutable v1 generations first, shadow existing reads, then migrate tenants in
batches. Rollback disables new signing and managed transformations and restores the prior authorized
read adapter. It must never make a bucket public, accept an old cache generation, bypass a hold, or
rewrite an immutable key.
