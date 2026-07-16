# ADR 0002: Resolve the requested “R3” storage target

- Status: proposed
- Date: 2026-07-15

## Context

The repository brief names “R3.” As of this scaffold, Cloudflare documents D1 for relational data and R2 for object storage; no matching Cloudflare R3 product was found. Cap also supports generic S3-compatible storage and Google Drive, so hard-coding R2 could create a parity regression.

## Proposed decision

Treat R2 as the provisional Cloudflare binding while all domain and application code depends on an `ObjectStore`/upload-broker port. Decide explicitly whether the final product is R2-only, R2 plus S3 compatibility, or a differently intended “R3” technology before production storage work begins.

## Consequences

The initial Worker uses a binding named `RECORDINGS`, not provider terminology. Storage-specific behavior remains at the adapter boundary. Issue 02 owns the decision, product-parity analysis, and naming cleanup.
