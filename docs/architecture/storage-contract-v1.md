# Storage contract v1

This document specifies the provider-neutral, locally testable storage core used by issues 02 and
18. ADR 0002 selects Cloudflare R2 and the `RECORDINGS` Worker binding for hosted Frame storage.
This contract does **not** implement or validate that R2 adapter and does not close either issue.

## Canonical key grammar

All keys use schema version 1, canonical UUID text, and a positive JSON-safe object revision.

Source object:

```text
tenants/<tenant_uuid>/videos/<video_uuid>/v1/<source_role>/r<revision>/<generated_name>
```

Derivative object or derivative manifest:

```text
tenants/<tenant_uuid>/videos/<video_uuid>/v1/<derivative_role>/source-r<revision>/p<profile_version>-<profile_sha256>/<generated_name>
```

The derivative revision is the immutable source revision. A normalized transform profile is a
strictly key-sorted `name=value` sequence separated by semicolons. One registered profile binds one
media output descriptor (role plus generated filename). The full SHA-256 of unambiguously
length-framed profile name, normalized profile, output role, and output filename is included in the
key; none of those values is exposed in the key. The profile version is a separate key component,
so identical settings under two registry versions cannot collide. The executor is deliberately excluded: an
equivalent Cloudflare Media and native GStreamer request therefore resolves to the same output key.
The manifest records which executor actually produced the bytes.

### Closed role and filename mapping

| Role | Generated filename | Source or derivative |
| --- | --- | --- |
| `source` | `source.<ext>` | source |
| `segment` | `segment-<8 digit index>.<ext>` | source |
| `thumbnail` | `thumbnail.<ext>` | derivative |
| `preview` | `preview.<ext>` | derivative |
| `spritesheet` | `spritesheet.<ext>` | derivative |
| `audio` | `audio.<ext>` | derivative |
| `export` | `export.<ext>` | derivative |
| `manifest` | `manifest.json` | profile-bound manifest (never a media output) |

An extension is 1–16 lowercase ASCII letters or digits. There is no API that accepts a user
basename. This prevents traversal, Unicode normalization ambiguity, hidden dot segments, and names
such as a project title or original local filename from entering a provider key. Parsing accepts
only a string that exactly reproduces the canonical generated form, so alternate revision padding,
uppercase UUIDs/extensions, extra path components, and role/filename disagreement fail closed.

`ScopedObjectKey` retains the tenant and video as typed values. Every port operation also receives a
tenant request context; mismatches return the same `not_found` class as an absent object. List is
always constrained to one exact tenant and video. Server-side copy is limited to the same tenant and
video. Application immutable-put orchestration applies this tenant fence before inspecting provider
capabilities or object-size limits and before invoking the adapter.

## Immutable derivative manifest

`DerivativeManifest` has schema version 1 and private fields with a validating constructor. It
records:

- source key, source revision (inside the key), and SHA-256;
- normalized transform profile, profile name/version, and derived SHA-256 fingerprint;
- actual executor (`cloudflare_media` or `native_gstreamer`);
- output key, SHA-256, byte count, and content type;
- positive attempt number and creation timestamp.

Construction and deserialization both reject an unknown executor, zero-size output, a source- or
manifest-shaped output, a derivative-shaped source, different tenant/video/revision scopes, a
profile fingerprint mismatch, an output key derived from another profile, or an unsupported
manifest version. Profile and manifest wire objects reject unknown fields. A manifest key is a
separate profile-bound artifact and can never satisfy the media-output invariant. The manifest has
no credential, signed URL, provider endpoint, or arbitrary metadata field.

## Provider-neutral object-store boundary

`ObjectStoreV1` negotiates an explicit versioned capability set before I/O. Capabilities cover
`put`, `head`, `get`, ranges, copy, delete, scoped list, create conditions, provider-version
conditions, and SHA-256 integrity, with declared object/range/page limits.

All media writes and copy destinations are create-only. There is no unconditional overwrite or
provider-version overwrite method for a `ScopedObjectKey`. Provider-version conditions fence a copy
source or a delete. If an expected provider version is supplied and the source/object is absent,
the result is `precondition_failed`; absence therefore cannot silently satisfy a version fence.
Unconditioned deletes return `deleted` or `already_absent`, making their retries idempotent.

A successful write result records the canonical key, exact byte count, content type, SHA-256,
cache policy, opaque provider version, opaque provider etag, last-modified time, and correlation ID.
Provider versions and etags are equality tokens only. In particular, the contract never interprets
an etag as a content checksum.

The stable failure classes are:

- `not_found`;
- `precondition_failed`;
- `throttled` (optional bounded retry delay);
- `unauthorized`;
- `quota_exceeded`;
- `timeout`;
- `integrity`;
- `unavailable`;
- `unsupported_capability`;
- `invalid_request`.

Only throttling, timeout, and unavailability are retryable. Adapter/provider diagnostics are not
part of this error and cannot be formatted through it.

The workspace still contains the earlier minimal `ObjectStore`/`AdvancedObjectStore` interfaces
used by pre-existing multipart and backfill scaffolding. This slice leaves those callers untouched
to avoid silently changing in-flight issue 19/20 work. `ObjectStoreV1` is the issue 18 contract for
new provider adapters; migrating or removing the earlier interfaces requires their callers to pass
this contract suite and is not evidence that an R2 adapter exists.

## Upload-broker boundary

`UploadBrokerV1` declares support independently for brokered single-put, direct single-put, and
multipart modes, plus its maximum object size and SHA-256 requirement. Application preflight checks
the store and broker capabilities and limits before invoking `begin`, so an unsupported request
cannot leave a broker session behind.

Broker plans bind a redacted upload ID to the exact key and tenant, mode/delivery shape, size,
content type, checksum, cache policy, expiry, and correlation ID. Application orchestration checks
every returned field before accepting a provider plan. Completion accepts only an object-store
receipt matching the bound write fields and is replay-safe. An owner abort removes a pending upload;
abort after completion is a no-op that preserves completion replay. Unknown and cross-tenant aborts
are intentionally indistinguishable no-ops. Same-origin broker paths reject traversal, query
strings, fragments, and protocol-relative paths. Direct authorization material is an explicitly
sensitive type: it requires HTTPS, rejects every ASCII control character and case-insensitive
duplicate header name, and redacts the URL and header values from `Debug`.

Multipart transfer mechanics, part sizing, resume state, and client UI remain issue 19 work. This
boundary only negotiates the mode and binds its final receipt.

## Deterministic local adapter

`DeterministicObjectStore` and `DeterministicUploadBroker` are provider-free contract fakes. They use
ordered in-memory state, monotonic fake provider versions/timestamps, atomic create conditions,
real SHA-256 verification, deterministic pagination, one-shot fault injection, and no network or
credential dependency. They prove the contract and application preflight behavior; they do not
claim R2 compatibility, durability, latency, or security posture.

Fault injection is limited to actual I/O paths (`put`, `head`, `get`, range, copy, delete, and
list), rather than capability-only enum values that no request consumes. Tenant authorization runs
before capabilities or queued failures, so a cross-tenant probe cannot learn a disabled capability
or consume a fault intended for an authorized request. Raw put/body/range bytes and upload paths are
redacted from debug output.

## Local issue 18 contract gaps

These are local contract-design gaps, not Cloudflare/R2 deployment gates, and this slice does not
claim to resolve them:

- avatar storage needs a user-scoped key grammar and ownership boundary; the current grammar is
  intentionally video-scoped;
- screenshots and any other legacy object roles still need an authoritative role inventory and a
  preserve/map/retire decision;
- arbitrary legacy metadata and tags still need a typed, bounded contract or an explicit retirement
  decision;
- a representative legacy key sample and deterministic old-to-v1 key mapping have not been attached
  or exercised.

These gaps must be reconciled with real legacy data before issue 18 can close, independently of the
protected/provider gates below.

## Compatibility and protected gates still pending

The Cap S3-compatible, MinIO, Google Drive, self-hosted, and user-owned bucket modes remain migration
inputs. No retained compatibility adapter is approved or implemented by this slice. Their final
preserve/change/defer/retire decisions, owners, migration impact, and rollback implications remain
an issue 02 product/security approval gate.

The following evidence is also still required before issues 02 or 18 can close:

- accepted approval record for ADR 0002, production/non-production bucket names and ownership, and
  the `RECORDINGS` binding configuration;
- security-owner approval of credentials, direct-upload signing, tenant isolation, Media
  Transformations access to private inputs, lifecycle, legal hold, residency, and data deletion;
- capability, compliance, residency, retention-cost, pricing, and egress models;
- an actual R2 Worker adapter and local plus hosted bucket contract reports for put, head, get,
  range, copy, delete, list, conditions, checksums, and provider-version/etag behavior;
- private R2-to-Media Transformations feasibility evidence;
- approved compatibility-adapter reports where parity is retained;
- production backfill/reconciliation, lifecycle/security, multipart, rollout, and rollback evidence
  owned by issues 19–21.

Until those protected/provider gates are attached, legacy reads remain authoritative and this v1
contract is a locally proven foundation only.
