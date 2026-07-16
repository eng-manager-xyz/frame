#!/usr/bin/env bash
set -euo pipefail

before="${1:?usage: release-change-plan.sh BEFORE_SHA [AFTER_SHA]}"
after="${2:-HEAD}"
zero_sha="0000000000000000000000000000000000000000"

if [[ "${before}" == "${zero_sha}" ]] || ! git cat-file -e "${before}^{commit}" 2>/dev/null; then
  changed="$(git ls-tree -r --name-only "${after}")"
else
  changed="$(git diff --name-only "${before}" "${after}")"
fi

worker_changed=false
web_changed=false
infra_changed=false

while IFS= read -r path; do
  [[ -z "${path}" ]] && continue
  case "${path}" in
    apps/control-plane/* | crates/application/* | crates/domain/* | crates/frame-client/* | crates/ports/* | fixtures/frame-api/* | Cargo.toml | Cargo.lock | rust-toolchain.toml | .github/workflows/production-gate.yml | scripts/ci/check-migrations.py | scripts/ci/package-release.sh | scripts/ci/verify-release-bundle.sh)
      worker_changed=true
      ;;
  esac
  case "${path}" in
    apps/web/* | crates/application/* | crates/domain/* | crates/frame-client/* | crates/ports/* | Cargo.toml | Cargo.lock | rust-toolchain.toml | render.yaml)
      web_changed=true
      ;;
  esac
  case "${path}" in
    infra/cloudflare-account/* | .github/workflows/cloudflare-account.yml | scripts/ci/install-terraform.sh)
      infra_changed=true
      ;;
  esac
done <<< "${changed}"

reason="worker=${worker_changed};web=${web_changed};account_infra=${infra_changed}"
printf 'worker_changed=%s\nweb_changed=%s\ninfra_changed=%s\nreason=%s\n' \
  "${worker_changed}" "${web_changed}" "${infra_changed}" "${reason}"

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    printf 'worker_changed=%s\n' "${worker_changed}"
    printf 'web_changed=%s\n' "${web_changed}"
    printf 'infra_changed=%s\n' "${infra_changed}"
    printf 'reason=%s\n' "${reason}"
  } >> "${GITHUB_OUTPUT}"
fi
