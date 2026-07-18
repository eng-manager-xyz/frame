# Cloudflare edge and account local evidence

Local evidence is credential-free and proves policy shape, not provider state.

```text
python3 -I scripts/ci/check-cloudflare-zone-contract.py
python3 -I scripts/ci/check-cloudflare-edge-policy.py
python3 -I scripts/ci/test-frame-cache-purge.py
scripts/ci/install-terraform.sh --check
terraform -chdir=infra/cloudflare-account fmt -check -recursive
terraform -chdir=infra/cloudflare-account init -backend=false -input=false -lockfile=readonly
terraform -chdir=infra/cloudflare-account validate
```

The executable cache matrix contains 20 host/method/path/auth/cookie/privacy/
range/upgrade/response cases. Exactly two fingerprinted GET/HEAD controls are
eligible for immutable caching; every dynamic or sensitive case bypasses. The
account checker proves a locked Cloudflare provider, private bucket
`prevent_destroy`, exact-origin CORS, abandoned multipart cleanup, and absence
of DNS/zone/ruleset/Worker ownership. Seven purge hazards are rejected.

On this workstation the Python policy/purge/zone checks pass. The
checksum-pinned Terraform 1.9.8 installer was run into an ephemeral directory;
the committed Cloudflare provider lock resolved v5.21.1, and credential-free
`fmt`, backend-disabled `init -lockfile=readonly`, and `validate` all passed.
No backend, provider credentials, remote state, plan, or mutation was used.

Protected evidence still required: portfolio-zone imports and semantic no-op,
legacy bootstrap retirement, remote plans/state backup-restore, DNS/TLS and
cache HIT traces, browser R2/CORS/provider behavior, scoped purge timing,
WAF/rate observation/enforcement, token scopes, drift alert, and rollback. No
production/cache/provider checkbox is closed by this local record.
