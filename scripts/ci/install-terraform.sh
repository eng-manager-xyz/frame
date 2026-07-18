#!/usr/bin/env bash
set -euo pipefail

readonly TERRAFORM_VERSION="1.9.8"

checksum_for() {
  case "$1" in
    linux_amd64) echo "186e0145f5e5f2eb97cbd785bc78f21bae4ef15119349f6ad4fa535b83b10df8" ;;
    linux_arm64) echo "f85868798834558239f6148834884008f2722548f84034c9b0f62934b2d73ebb" ;;
    darwin_amd64) echo "be591e8c59c49d0cfbc7664d24910a4b43840b89d0a4bbca662149bbf0397e91" ;;
    darwin_arm64) echo "873d7b925d08578fb6bb9c12c7cd92ae73e289e07c360f2fdd69f9036b7baaab" ;;
    *) return 1 ;;
  esac
}

if [[ "${1:-}" == "--check" ]]; then
  for platform in linux_amd64 linux_arm64 darwin_amd64 darwin_arm64; do
    checksum="$(checksum_for "${platform}")"
    [[ "${checksum}" =~ ^[0-9a-f]{64}$ ]] || {
      echo "invalid pinned checksum for ${platform}" >&2
      exit 1
    }
  done
  echo "Terraform ${TERRAFORM_VERSION} platform checksum table is complete."
  exit 0
fi

case "$(uname -s)" in
  Linux) os="linux" ;;
  Darwin) os="darwin" ;;
  *) echo "unsupported operating system: $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64) arch="amd64" ;;
  arm64 | aarch64) arch="arm64" ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

platform="${os}_${arch}"
expected="$(checksum_for "${platform}")"
archive="terraform_${TERRAFORM_VERSION}_${platform}.zip"
url="https://releases.hashicorp.com/terraform/${TERRAFORM_VERSION}/${archive}"
destination="${1:-${RUNNER_TEMP:-/tmp}/terraform-bin}"
temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT

curl --fail --location --silent --show-error \
  --proto '=https' --tlsv1.2 --retry 3 --retry-all-errors \
  --output "${temporary}/${archive}" "${url}"

if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "${temporary}/${archive}" | awk '{print $1}')"
else
  actual="$(shasum -a 256 "${temporary}/${archive}" | awk '{print $1}')"
fi

if [[ "${actual}" != "${expected}" ]]; then
  echo "Terraform archive checksum mismatch for ${platform}" >&2
  exit 1
fi

mkdir -p "${destination}"
unzip -q "${temporary}/${archive}" -d "${temporary}/unpacked"
install -m 0755 "${temporary}/unpacked/terraform" "${destination}/terraform"

if [[ -n "${GITHUB_PATH:-}" ]]; then
  printf '%s\n' "${destination}" >> "${GITHUB_PATH}"
fi

"${destination}/terraform" version
