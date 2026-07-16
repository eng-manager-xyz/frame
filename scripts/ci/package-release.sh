#!/usr/bin/env bash
set -euo pipefail

output="${1:-target/frame-release}"
release_sha="${RELEASE_SHA:-$(git rev-parse HEAD)}"
web_binary="${WEB_BINARY:-target/release/frame-web}"
worker_bundle="${WORKER_BUNDLE:-target/wrangler-release}"
web_assets="${WEB_ASSETS:-apps/web/dist}"

[[ "${release_sha}" =~ ^[0-9a-f]{40}$ ]] || {
  echo "RELEASE_SHA must be a full Git commit SHA" >&2
  exit 1
}
[[ -x "${web_binary}" ]] || {
  echo "missing release web binary: ${web_binary}" >&2
  exit 1
}
[[ -d "${worker_bundle}" ]] || {
  echo "missing Worker dry-run bundle: ${worker_bundle}" >&2
  exit 1
}
[[ -f "${web_assets}/manifest.json" ]] || {
  echo "missing web hydration assets: ${web_assets}" >&2
  exit 1
}
python3 -I scripts/ci/check-web-hydration-bundle.py --dist "${web_assets}" >/dev/null
[[ "$(basename "${worker_bundle}")" == "wrangler-release" ]] || {
  echo "Worker bundle directory must be named wrangler-release" >&2
  exit 1
}
[[ -f "${worker_bundle}/index.js" ]] || {
  echo "Worker dry-run bundle is missing its index.js entrypoint" >&2
  exit 1
}
if find "${worker_bundle}" -type l -print -quit | grep -q .; then
  echo "Worker dry-run bundle may not contain symbolic links" >&2
  exit 1
fi

mkdir -p "${output}"
if find "${output}" -mindepth 1 -maxdepth 1 -print -quit | grep -q .; then
  echo "release output must be empty: ${output}" >&2
  exit 1
fi

contract_major="$(sed -nE 's/^pub const CONTRACT_MAJOR: u16 = ([0-9]+);/\1/p' crates/frame-client/src/dto.rs)"
[[ "${contract_major}" =~ ^[0-9]+$ ]] || {
  echo "unable to read frame-client CONTRACT_MAJOR" >&2
  exit 1
}

cp "${web_binary}" "${output}/frame-web"
cp -R "${web_assets}" "${output}/web-dist"
tar -C "$(dirname "${worker_bundle}")" -czf "${output}/frame-worker.tar.gz" wrangler-release
if find "${output}/web-dist" -type l -print -quit | grep -q .; then
  echo "web hydration bundle may not contain symbolic links" >&2
  exit 1
fi
metadata_raw="$(mktemp)"
trap 'rm -f "${metadata_raw}"' EXIT
cargo metadata --locked --format-version 1 > "${metadata_raw}"
jq '{
  schema_version: 1,
  generated_from: "cargo metadata --locked --format-version 1",
  packages: ([.packages[] | {
    name,
    version,
    source,
    checksum,
    license,
    repository,
    dependencies: [.dependencies[] | {name, req, source, kind, optional, target}]
  }] | sort_by(.name, .version, (.source // "workspace")))
}' "${metadata_raw}" > "${output}/cargo-metadata.json"
python3 scripts/ci/generate-cyclonedx.py \
  --metadata "${metadata_raw}" \
  --cargo-lock Cargo.lock \
  --workspace-name frame \
  --require-registry-checksums \
  --output "${output}/frame.cdx.json"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

web_sha="$(sha256_file "${output}/frame-web")"
web_assets_sha="$(sha256_file "${output}/web-dist/manifest.json")"
worker_sha="$(sha256_file "${output}/frame-worker.tar.gz")"
metadata_sha="$(sha256_file "${output}/cargo-metadata.json")"
sbom_sha="$(sha256_file "${output}/frame.cdx.json")"
migration_level="$(find apps/control-plane/migrations -maxdepth 1 -type f -name '*.sql' -print | sort | tail -n 1 | xargs basename)"

jq -n \
  --arg schema_version "1" \
  --arg git_sha "${release_sha}" \
  --arg contract_major "${contract_major}" \
  --arg migration_level "${migration_level}" \
  --arg render_authority "git-checks-pass" \
  --arg web_sha256 "${web_sha}" \
  --arg web_assets_sha256 "${web_assets_sha}" \
  --arg worker_sha256 "${worker_sha}" \
  --arg cargo_metadata_sha256 "${metadata_sha}" \
  --arg sbom_sha256 "${sbom_sha}" \
  '{
    schema_version: ($schema_version | tonumber),
    git_sha: $git_sha,
    contract_major: ($contract_major | tonumber),
    migration_level: $migration_level,
    render_authority: $render_authority,
    portfolio_consumer_sha: null,
    artifacts: {
      web: {path: "frame-web", sha256: $web_sha256},
      web_assets: {path: "web-dist/manifest.json", layout: "web-dist", sha256: $web_assets_sha256},
      worker: {path: "frame-worker.tar.gz", sha256: $worker_sha256},
      cargo_metadata: {path: "cargo-metadata.json", sha256: $cargo_metadata_sha256},
      sbom: {path: "frame.cdx.json", format: "CycloneDX", spec_version: "1.6", sha256: $sbom_sha256}
    }
  }' > "${output}/release-manifest.json"

(
  cd "${output}"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum frame-web frame-worker.tar.gz cargo-metadata.json frame.cdx.json release-manifest.json > SHA256SUMS
    find web-dist -type f -print | LC_ALL=C sort | while IFS= read -r asset; do sha256sum "${asset}"; done >> SHA256SUMS
  else
    shasum -a 256 frame-web frame-worker.tar.gz cargo-metadata.json frame.cdx.json release-manifest.json > SHA256SUMS
    find web-dist -type f -print | LC_ALL=C sort | while IFS= read -r asset; do shasum -a 256 "${asset}"; done >> SHA256SUMS
  fi
)

echo "Packaged release ${release_sha} with contract major ${contract_major} and migration ${migration_level}."
