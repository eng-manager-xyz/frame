# Frame Cloudflare account infrastructure

This state owns only Frame account-scoped R2 resources: the private recordings
bucket, exact-origin direct-transfer CORS, and abandoned multipart cleanup. It
does not own DNS, Worker Routes, cache, WAF, rate limits, or any shared zone
phase. Those resources belong to the single `engmanager.xyz` zone state
described in `../cloudflare-zone/README.md`.

The Cloudflare provider is pinned because R2 CORS/lifecycle shapes are part of
the upload and privacy contract. Provider credentials come only from
`CLOUDFLARE_API_TOKEN`; never place a token in tfvars, source, a plan artifact,
or a pull-request job.

## State and authority

- Use a remote encrypted backend with locking and versioning. The backend
  configuration is supplied during trusted initialization and is intentionally
  absent from source.
- Staging and production use separate state keys, bucket names, credentials,
  and exact origins.
- Import an existing bucket before the first plan. The CORS and lifecycle
  resources currently do not support Terraform import, so the first protected
  plan must compare their live values and be reviewed as an intentional
  authority adoption.
- `prevent_destroy` protects the bucket. Data deletion is manifest-driven in
  the application; Terraform is not a media retention tool.
- Wrangler binds the resulting bucket name but does not provision or delete
  it. Dashboard edits are drift and must be reverted through this state.
- Back up each encrypted/versioned remote state before apply. Restore only to
  an isolated backend first, refresh without mutation, compare resource IDs,
  and require a reviewed plan before promoting that state version.
- The account token needs only the exact R2 bucket/CORS/lifecycle capabilities;
  it receives no zone DNS/ruleset, Worker deploy, D1, Media, or unrelated
  bucket permission. Rotate through protected environments and audit state and
  provider history after suspected disclosure.

## Trusted workflow

```sh
terraform init -backend-config="$BACKEND_CONFIG"
terraform fmt -check -recursive
terraform validate
terraform plan -lock-timeout=5m -out=frame.tfplan -var-file=production.tfvars
terraform show -json frame.tfplan > frame-plan.json
terraform apply -lock-timeout=5m frame.tfplan
terraform plan -detailed-exitcode -var-file=production.tfvars
```

The plan JSON is retained briefly as a redacted release artifact. An apply is
manual, protected, serialized, and restricted to the exact reviewed plan. A
post-apply probe checks allowed/disallowed origins, methods, headers, ranges,
validators, expiry, cross-tenant rejection, and confirms the bucket is not
public.

The application supports an explicit `direct` upload intent in addition to
the default `brokered` transfer. Direct intent fails closed unless the Worker
has `FRAME_R2_BUCKET_NAME` plus protected `FRAME_R2_ACCOUNT_ID`,
`FRAME_R2_ACCESS_KEY_ID`, and `FRAME_R2_SECRET_ACCESS_KEY` bindings. The signing
credential must be restricted to object-write access for this bucket; the
Worker binding performs verification, promotion, and cleanup. Secret material
is never supplied to Render or a browser; the browser receives only one
five-minute capability for its random private staging key.

The CORS allowlist intentionally includes the headers bound into that
capability: `content-length`, `content-type`, `if-none-match`,
`x-amz-checksum-sha256`, and `x-amz-meta-frame-sha256`. Changing this list must
run the wrong-origin/header denial matrix as well as the valid direct upload.
CORS and local signer tests do not prove hosted R2 SigV4/checksum behavior or a
pre-storage byte cap; those remain protected provider evidence in
`docs/operations/cloudflare-cache-security.md`.
