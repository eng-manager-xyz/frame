# Business data v1 architecture

Status: locally implemented; protected provider execution remains promotion evidence.

Source reference: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`, primarily `packages/database/schema.ts` and the referenced storage, video, organization, public-share, and developer-credit behavior.

## Boundary

`frame-domain::business` owns provider-neutral values and invariants. `frame-ports::business` is the repository capability boundary. `frame-application::business` re-evaluates policy, document mutability, exact accounting, and canonical semantic fingerprints before adapter I/O. `D1BusinessRepository` translates those capabilities into parameterized, authority-first D1 batches. No provider URL, plaintext credential, payment object, or JavaScript binding type crosses the domain, port, or application boundary.

The organization is the tenant boundary. `BusinessScope::new` requires the tenant UUID and organization UUID to be identical. Mutations carry identity, session, organization, membership, authority, and resource revision fences. D1 checks those fences in statement one; a trigger abort makes the complete batch atomic on a stale or unauthorized request.

## Source mapping

The migration persists the same inventory in `business_source_table_map_v1`, so drift can be queried rather than inferred from documentation.

| Cap table | v1 aggregate / target | Disposition |
| --- | --- | --- |
| `videos` | video metadata plus operation receipts | retained |
| `video_edits` | bounded versioned edit document | retained |
| `shared_videos` | tenant-scoped share | retained |
| `comments` | tenant-scoped user or anonymous comment | retained |
| `notifications` | notification plus transactional outbox | retained |
| `messenger_conversations` | legacy quarantine | excluded, fail closed |
| `messenger_messages` | legacy quarantine | excluded, fail closed |
| `messenger_support_emails` | legacy quarantine | excluded, fail closed |
| `s3_buckets` | normalized storage integration | retained through compatibility import |
| `storage_integrations` | encrypted provider capability | retained |
| `storage_objects` | immutable-key object manifest | retained |
| `video_uploads` | ordered upload lifecycle | retained |
| `imported_videos` | ordered import lifecycle | retained |
| `developer_apps` | tenant-scoped developer app | retained |
| `developer_app_domains` | normalized app domain | retained |
| `developer_api_keys` | digest plus display prefix only | retained |
| `developer_videos` | bounded developer metadata | retained |
| `developer_credit_accounts` | reconciled materialized balance | retained |
| `developer_credit_transactions` | append-only transaction ledger | retained |
| `developer_daily_storage_snapshots` | idempotent daily snapshot | retained |

The pinned Cap schema contains exactly 20 tables in this Issue-15 inventory.
`usage_ledger` is a Frame-derived auditable aggregate, not a 21st Cap source
table; `business_derived_aggregate_map_v1` records that provenance separately.
The credential-free production-plan contract in
`fixtures/etl/v1/cap-business-plan-contract.json` enumerates all 56 NanoID
identifier occurrences and applies the same option-free
`cap_nanoid_uuid_v1` UUIDv8 transform to every PK/FK position.
`fixtures/parity/v1/business-cap-schema-v1.json` freezes every source column
from that pinned schema file, its SHA-256, and 39 intentional source-to-Frame
drifts. The isolated contract test rejects a missing source table, identifier,
target column, drift rationale, transform, or changed source-column digest.

The three messenger tables intentionally have no product read or write capability. Migration 0011 inventories legacy rows in `business_messenger_legacy_quarantine_v1`, derives an organization only when the legacy relationship has one unambiguous active tenant, assigns a bounded 30-day purge deadline, and rejects new inserts and updates. The retention service has one administrative purge path: three tenant/deadline-guarded cleanup statements, the quarantine transition, and the raw-row absence postcondition execute in one D1 batch. The expand migration itself contains no destructive trigger. Ambiguous or unclassified rows remain fail closed and require a separately approved disposition. Product reintroduction requires a new reviewed contract.

## Versioned documents

Metadata, edits, notifications, outbox events, and developer metadata use `VersionedBusinessDocument`.

- Canonical JSON has sorted object keys, no insignificant whitespace, no duplicate-key representation, integral JSON-safe numbers, a maximum depth of 16, and a maximum of 16,384 nodes.
- Video, event, and developer documents are at most 64 KiB. Edit documents are at most 1 MiB.
- `schema_version = 1` is readable and writable. A canonical unknown positive version is `ReadOnlyPreserve`: it can be exported and retained byte-for-byte but cannot be rewritten by an older service.
- Every accepted document has a SHA-256 checksum. D1 stores the version and checksum beside the canonical document. Repository reads recompute and compare both.

Comment kinds are closed to text or bounded whitespace-free emoji reactions. Timeline values use integer microseconds rather than floating-point JSON. This avoids client/runtime normalization differences. Space shares require a same-organization live folder; organization and public-link shares forbid a folder.

## Collaboration and ordered workflows

Private videos are readable by the owner or organization owner/admin. Organization, unlisted, and public videos are readable by an active same-tenant member; only unlisted/public videos are visible anonymously. Anonymous comments require a keyed digest and the command author digest must equal the current principal digest. Missing, cross-tenant, private, and unauthorized resources collapse to the same denial.

Outbox, import, and upload events use a per-aggregate sequence plus semantic fingerprint. Every
new lifecycle starts at sequence zero with the single canonical
`frame-business-ordered-lifecycle-initial-v1` fingerprint:

- lower sequence: stale and ignored;
- equal sequence plus equal fingerprint: duplicate;
- equal sequence plus different fingerprint: conflict;
- next sequence: apply through a checked transition;
- future sequence: persist as deferred and re-evaluate after the gap closes.

The event inbox updates a previously deferred disposition to applied when the exact fingerprint becomes contiguous. A changed payload at the same sequence aborts.
An immutable `accepted` receipt does not prevent convergence: replaying that
same key after the gap closes executes the deferred transition and returns the
original receipt unchanged.
A failed import requires one bounded lowercase `RedactedFailureClass`; every
nonfailed import state forbids it, preventing provider messages and URLs from
entering durable workflow state.

## Storage and derivatives

An object manifest contains tenant, integration, optional video, internal relative object key, role, object version, state, byte size, content type, checksum, and timestamps. D1 verifies integration, video, and object scopes. Immutable identity fields cannot change during an update.

`business_derivative_manifests_v1` is the common completion boundary for managed and native execution. It records executor, source object and version, transform profile and version, output role/key/object/checksum/content type, lifecycle state, usage units, microcredit cost, revision, and a bounded redacted failure class. Succeeded jobs require a matching same-tenant output manifest; failed jobs require a redacted class. Neither path stores a signed/private media URL or binding-specific object.

## Developer and accounting data

Developer apps, normalized domains, and developer videos have typed repository operations with tenant/revision postconditions. External developer-video user identities persist only as SHA-256 digests, and optional metadata uses the same bounded versioned-document contract. API keys persist a SHA-256 digest, non-secret display prefix, kind, timestamps, and revocation state. The schema has no encrypted or plaintext recoverable key column.

Credit transactions are immutable and sequence exactly from the account. Before-insert validation proves `balance_after = current_balance + amount`, rejects overdraw and sequence gaps, and an after-insert trigger advances the materialized balance in the same transaction. Usage entries are immutable, semantically idempotent, and resolve their app/video/job back to the same organization. Daily storage snapshots are keyed by app and UTC day with a source checksum.

The balance is a cache of the append-only transaction stream, never an independent source of truth. Reconciliation replays transactions, compares the final balance, sums usage charges by reference, and checks daily snapshot checksums.

Owner-authorized exports return bounded, typed rows plus the manifest; they do
not stop at aggregate counts. The deterministic manifest checksum covers the
ordered class counts, every exported class/subject/document tuple, and the
source revision. Delete execution binds the subject to the tenant,
checks the legal hold in the same batch, and performs the class-specific
tombstone, purge, or cryptographic erasure before marking the request complete.
Deleting a credit or usage fact never mutates the original row: an exact,
balanced `adjustment` transaction referencing the deletion is appended and the
account sequence advances atomically.

## Rollout and rollback

Migration 0011 is expand-only and dirty-safe. Existing rows receive nullable audit metadata or safe defaults. Migrations 0016–0019 install the bounded enforcement phases, migration 0020 installs the D1-compatible integrity view, and migration 0021 restores the aggregate-repository compatibility trigger after every new outbox field exists. `business_source_integrity_v1` exposes legacy rows missing canonical checksums, ledger sequences, operation IDs, or scope. Those findings block write cutover but do not make the migration fail or silently normalize financial history. The split preserves ordered semantics while keeping each file below D1's compound-expression ceiling.

Roll out by aggregate: audit, backfill canonical metadata, compare read parity, enable tenant writes, then reconcile. Financial and developer aggregates require independent approval after a zero-drift reconciliation. Rollback disables the new write path and returns reads to the compatibility source; immutable receipts, ledgers, and audit records are retained. Schema contraction is a later migration after every finding and deferred event is resolved.
