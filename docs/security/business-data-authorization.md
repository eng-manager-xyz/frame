# Business data authorization and privacy

## Authorization order

Every repository read and mutation begins with `business_repository_assertions_v1`. A successful user assertion requires a current active user, identity revision, session version, active organization, active membership, exact authority revisions, and the role class required by the action. Object actions additionally bind the video ID and owner/privacy policy. Anonymous authority is limited to an exact unlisted/public commentable video and a 64-character keyed digest.

The assertion trigger aborts a false predicate. D1 batches are atomic, so the assertion insert, aggregate write, postcondition, operation receipt, audit event, and assertion cleanup either all commit or all roll back. A denial cannot leave tenant state, a receipt, or an assertion row behind.

Reads use the same first-statement pattern. Video reads bind the requested video into the assertion. Private content requires the resource owner or organization owner/admin. Missing, deleted, cross-tenant, and unauthorized IDs return the same public denial.

Tenant export, credit-account reads, and legal-hold reads use a distinct
owner-only assertion; they never reuse the blank-video member read predicate.
Notification reads bind the authenticated recipient in the data query. Every
data-handling mutation runs a class-specific subject assertion before retention
or deletion, so an ID from another organization cannot be used to create a
request or compensation.

## Policy matrix

| Action | Anonymous | Member/viewer | Resource owner | Org admin | Org owner |
| --- | --- | --- | --- | --- | --- |
| Read public/unlisted | allow | allow | allow | allow | allow |
| Read organization | deny | allow | allow | allow | allow |
| Read private | deny | deny | allow | allow | allow |
| Create comment | public/unlisted with keyed digest | readable video when comments enabled | allow | allow | allow |
| Edit/share video | deny | deny unless resource owner | allow | allow | allow |
| Manage notification | deny | deny | deny | allow | allow |
| Read/mark own notification | deny | allow | allow | allow | allow |
| Storage/import/developer | deny | deny | deny | allow | allow |
| Credit/usage/export/delete | deny | deny | deny | deny | allow |

Comment deletion is limited to the comment author or organization owner/admin. The repository stores either a user ID or an anonymous keyed digest, never both. The application and D1 adapter independently require the command author to match the current principal.

## Semantic idempotency and receipts

The application and D1 adapter independently recompute a length-framed SHA-256 fingerprint over action, tenant, principal kind, principal subject, idempotency key, subject, and the complete serialized typed payload. The application comparison covers every byte without data-dependent early exit. Operation receipts are unique by organization, principal, and idempotency key. Reuse with another action, subject, or any changed persisted field conflicts.

Receipt lookup is preceded by current-principal authorization and filters by the exact current principal. A user cannot read another user’s receipt. Anonymous principals can retrieve only a receipt associated with their keyed digest and an otherwise authorized public resource. Operation receipts and audit events are immutable.

Outbox and usage logical idempotency keys are domain-separated with the tenant
and purpose before reaching legacy globally unique columns. The same caller key
therefore succeeds independently in two organizations, while a same-tenant
replay remains a conflict-safe duplicate.

## Secret and URL handling

- Developer API keys are accepted only as digests at this layer. Only a short display prefix may be returned.
- Storage provider configuration is ciphertext with redacted `Debug`; it is exposed only to a provider adapter.
- Object manifests contain an internal validated relative key, never an HTTP URL, signed query, cookie, token, or provider upload-session URL.
- Audit events hash principal and subject values and contain stable action/outcome codes.
- Failure classes are lowercase stable codes of at most 64 bytes. Provider messages, stack traces, object URLs, and credentials are not stored.
- Domain, port, and application business modules contain no `JsValue` or binding-specific type.

## Retention and legal hold

`business_data_handling_policies_v1` has one explicit row for every data class. Storage credentials and API keys are never exportable and use cryptographic erasure. Credit and usage history is append-only; correction uses a compensating entry. Active legal holds block deletion for hold-capable classes. Receipt, ledger, and audit immutability prevents a data-delete workflow from rewriting financial history.

The compensation predicate binds the original row, tenant, account, inverse
amount or usage charge, next sequence, next balance, adjustment type, and
deterministic deletion reference before the new ledger row is inserted.

Messenger remains excluded fail closed. Legacy messages are quarantined for bounded purge and are not available through a product read capability. Administrative deletion requires the exact `source_table:source_id`, the derived organization, an elapsed purge deadline, and one operation id; guarded raw deletion, quarantine transition, and the absence postcondition share one D1 batch. Rows without an unambiguous organization cannot pass that assertion. The migration does not fabricate consent or silently convert those records into product data.
