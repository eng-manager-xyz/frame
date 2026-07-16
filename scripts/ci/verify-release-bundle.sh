#!/usr/bin/env bash
set -euo pipefail

bundle="${1:-target/downloaded-release}"
expected_sha="${EXPECTED_SHA:?EXPECTED_SHA must be a full Git commit SHA}"

[[ "${expected_sha}" =~ ^[0-9a-f]{40}$ ]] || {
  echo "EXPECTED_SHA must be a full Git commit SHA" >&2
  exit 1
}

for required in frame-web frame-worker.tar.gz cargo-metadata.json frame.cdx.json release-manifest.json SHA256SUMS; do
  [[ -f "${bundle}/${required}" ]] || {
    echo "release bundle is missing ${required}" >&2
    exit 1
  }
done

(
  cd "${bundle}"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum --check --strict SHA256SUMS
  else
    shasum -a 256 --check SHA256SUMS
  fi
)

jq -e --arg sha "${expected_sha}" '
  .schema_version == 1 and
  .git_sha == $sha and
  .contract_major == 1 and
  .render_authority == "git-checks-pass" and
  (.migration_level | test("^[0-9]{4}_[a-z0-9_]+\\.sql$")) and
  .artifacts.web.path == "frame-web" and
  .artifacts.worker.path == "frame-worker.tar.gz" and
  .artifacts.cargo_metadata.path == "cargo-metadata.json" and
  .artifacts.sbom.path == "frame.cdx.json" and
  .artifacts.sbom.format == "CycloneDX" and
  .artifacts.sbom.spec_version == "1.6"
' "${bundle}/release-manifest.json" >/dev/null

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

for artifact in web worker cargo_metadata sbom; do
  path="$(jq -r --arg artifact "${artifact}" '.artifacts[$artifact].path' "${bundle}/release-manifest.json")"
  expected="$(jq -r --arg artifact "${artifact}" '.artifacts[$artifact].sha256' "${bundle}/release-manifest.json")"
  actual="$(sha256_file "${bundle}/${path}")"
  if [[ "${actual}" != "${expected}" ]]; then
    echo "release manifest checksum mismatch for ${artifact}" >&2
    exit 1
  fi
done

jq -e '
  .bomFormat == "CycloneDX" and
  .specVersion == "1.6" and
  .version == 1 and
  (.components | type == "array" and length > 0) and
  (.dependencies | type == "array" and length > 0)
' "${bundle}/frame.cdx.json" >/dev/null

python3 - "${bundle}/frame-worker.tar.gz" <<'PY'
import pathlib
import sys
import tarfile

archive = pathlib.Path(sys.argv[1])
names = set()
with tarfile.open(archive, "r:gz") as handle:
    for member in handle.getmembers():
        path = pathlib.PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts or member.name in names:
            raise SystemExit("unsafe or duplicate path in Worker archive")
        if not (member.isfile() or member.isdir()):
            raise SystemExit("Worker archive may contain only regular files and directories")
        names.add(member.name)
if "wrangler-release/index.js" not in names:
    raise SystemExit("Worker archive is missing wrangler-release/index.js")
PY

if grep -Eiq '(authorization:|bearer[[:space:]]|x-amz-signature|signed[_-]?url|cookie:|password|api[_-]?token|client[_-]?secret)' \
  "${bundle}/release-manifest.json" "${bundle}/SHA256SUMS"; then
  echo "release metadata contains a forbidden credential marker" >&2
  exit 1
fi

echo "Verified immutable release bundle for ${expected_sha}."
