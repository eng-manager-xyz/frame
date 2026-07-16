# ADR 0002: Use Cloudflare R2 as canonical object storage

- Status: accepted
- Date: 2026-07-15

## Context

Frame needs durable storage for source recordings, segments, thumbnails, previews, exports, and manifests. The Cloudflare control plane can access R2 through a first-class Worker binding, and Cloudflare Media Transformations can consume private R2 bodies without making source media public. Cap also has S3-compatible, MinIO, user-owned bucket, and Google Drive modes whose product disposition must remain explicit during migration.

## Decision

Use Cloudflare R2 as Frame's canonical hosted object store through the `RECORDINGS` binding. Store media under immutable, tenant-scoped, versioned keys and persist checksums and output manifests in D1. Domain and application code depend on provider-neutral `ObjectStore` and upload-broker ports so local fakes, contract tests, and any separately approved self-hosted or bring-your-own-storage adapters do not leak provider behavior into the domain.

Provider selection is settled. Issue 02 records the compatibility matrix for legacy S3-compatible, MinIO, user-owned bucket, and Google Drive modes; issue 18 implements the R2 adapter and key contract.

## Capability, compliance, residency, and cost model

Every environment fills this model from a dated provider quote and a measured
representative workload. The repository fixes variables and admission rules;
it deliberately does not freeze a price or invent approval.

| Input | Required measurement / decision | Admission rule |
|---|---|---|
| logical source and derivative bytes by retention class | p50/p95/max recording size, derivative multiplier, daily ingest, retained days | capacity includes incomplete multipart and reconciliation headroom |
| Class A / write-like operations | PUT, multipart create/part/complete/abort, copy, list and lifecycle counts | estimate normal, retry and backfill peaks separately |
| Class B / read-like operations | HEAD, GET, range, manifest and verification counts | player range amplification is measured, not assumed |
| egress and provider-to-provider transfer | browser delivery, migration source, restore and Media paths | each non-zero paid path has an owner and monthly ceiling |
| jurisdiction and location hint | tenant residency requirement, selected R2 location, processing/fallback regions | an unsupported residency fails before upload |
| retention, hold and deletion | source/output/backup windows and legal-hold overrides | lifecycle never deletes a held or unmanifested object |
| durability and recovery | provider commitment, manifest/checksum coverage, backup/restore RPO/RTO | missing restore evidence blocks authority cutover |
| compliance and access | data classification, encryption, token scopes, audit/incident needs | no public bucket or account-wide application credential |

For a scenario `s`, the monthly estimate is recorded as:

```text
stored_gb_month(s) * approved_storage_rate
+ class_a_millions(s) * approved_class_a_rate
+ class_b_millions(s) * approved_class_b_rate
+ paid_egress_gb(s) * approved_egress_rate
+ bounded_retry_and_backfill_reserve(s)
```

Required scenarios are a 60-second recording, a representative p95 recording,
the approved maximum recording, normal monthly traffic, one full integrity
scan, one restore, and the production-scale backfill rehearsal. The protected
release record supplies provider date/version, numeric inputs/rates, resulting
monthly totals, quota headroom, residency/compliance review, and accountable
approvals. Missing inputs fail the gate; they are not treated as zero.

Legacy S3-compatible, MinIO, user-owned bucket, self-hosted, and Drive modes
remain migration inputs. A retained adapter needs an explicit capability row,
credential boundary, residency/cost impact, owner, and rollback evidence. In
the absence of that approval, Frame permits read/export/backfill only and
rejects new hosted writes before transfer.

## Consequences

R2 binding semantics, multipart limits, conditional operations, range access, lifecycle, residency, access control, and cost become production design inputs. Provider etags must not be assumed to be content hashes. Compatibility adapters may expose fewer capabilities and must fail before an upload begins when a required operation is unsupported. Reversing this decision requires a new ADR and an object migration plan, not a domain-model rewrite.

## References

- [R2 Worker binding API](https://developers.cloudflare.com/r2/api/workers/workers-api-reference/)
- [R2 presigned URLs](https://developers.cloudflare.com/r2/api/s3/presigned-urls/)
