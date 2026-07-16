#!/usr/bin/env bash
set -euo pipefail

origin="${1:?usage: smoke-canonical-domain.sh ORIGIN [REPORT_PATH]}"
report="${2:-${REPORT_PATH:-target/canonical-smoke.json}}"
expected_release="${3:-${EXPECTED_RELEASE_SHA:-}}"

case "${origin}" in
  https://frame.engmanager.xyz | https://frame-staging.engmanager.xyz) ;;
  *) echo "origin is not in the canonical smoke allowlist" >&2; exit 1 ;;
esac
if [[ -n "${expected_release}" && ! "${expected_release}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "expected release must be a full Git commit SHA" >&2
  exit 1
fi

temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT
mkdir -p "$(dirname "${report}")"

request() {
  local name="$1"
  local path="$2"
  local headers="${temporary}/${name}.headers"
  local body="${temporary}/${name}.body"
  local status
  status="$(curl --fail-with-body --silent --show-error \
    --proto '=https' --tlsv1.2 --retry 2 --retry-all-errors \
    --connect-timeout 5 --max-time 20 \
    --dump-header "${headers}" --output "${body}" --write-out '%{http_code}' \
    "${origin}${path}")"
  printf '%s' "${status}" > "${temporary}/${name}.status"
}

assert_not_shared_cache() {
  local name="$1"
  local cache
  cache="$(awk 'BEGIN {IGNORECASE=1} /^cf-cache-status:/ {gsub("\\r", "", $2); print toupper($2)}' "${temporary}/${name}.headers" | tail -n 1)"
  case "${cache}" in
    DYNAMIC | BYPASS) ;;
    *) echo "${name} must report Cloudflare DYNAMIC or BYPASS, got ${cache:-missing}" >&2; exit 1 ;;
  esac
}

request web_ready "/health/ready"
request api_health "/api/v1/health"

[[ "$(cat "${temporary}/web_ready.status")" == "200" ]] || {
  echo "web readiness did not return HTTP 200" >&2
  exit 1
}
[[ "$(cat "${temporary}/api_health.status")" == "200" ]] || {
  echo "API health did not return HTTP 200" >&2
  exit 1
}

jq -e '.service == "frame-web" and .status == "ready" and (.release | test("^[A-Za-z0-9._-]{1,64}$"))' \
  "${temporary}/web_ready.body" >/dev/null
actual_release="$(jq -r '.release' "${temporary}/web_ready.body")"
if [[ -n "${expected_release}" && "${actual_release}" != "${expected_release}" ]]; then
  echo "canonical web release has not reached the expected Git SHA" >&2
  exit 1
fi

jq -e '.api_version.major == 1 and .service == "frame" and .status == "ok"' \
  "${temporary}/api_health.body" >/dev/null
assert_not_shared_cache api_health

content_type="$(awk 'BEGIN {IGNORECASE=1} /^content-type:/ {sub(/^[^:]+:[[:space:]]*/, ""); gsub("\\r", ""); print tolower($0)}' "${temporary}/api_health.headers" | tail -n 1)"
[[ "${content_type}" == application/json* ]] || {
  echo "API health must return application/json" >&2
  exit 1
}
cache_control="$(awk 'BEGIN {IGNORECASE=1} /^cache-control:/ {sub(/^[^:]+:[[:space:]]*/, ""); gsub("\\r", ""); print tolower($0)}' "${temporary}/api_health.headers" | tail -n 1)"
if [[ "${cache_control}" != *no-store* && "${cache_control}" != *private* ]]; then
  echo "API health must explicitly prevent shared caching" >&2
  exit 1
fi
if grep -Eiq '^set-cookie:' "${temporary}/api_health.headers"; then
  echo "API health must not set a cookie" >&2
  exit 1
fi

if grep -Eiq '(x-amz-signature|signed[_-]?url|authorization|object[_-]?key|bucket[_-]?name|database[_-]?id)' \
  "${temporary}/api_health.body"; then
  echo "public health response contains a forbidden private marker" >&2
  exit 1
fi

jq -n \
  --arg origin "${origin}" \
  --arg checked_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg web_ready "$(cat "${temporary}/web_ready.status")" \
  --arg api_health "$(cat "${temporary}/api_health.status")" \
  --arg actual_release "${actual_release}" \
  --arg expected_release "${expected_release}" \
  '{
    schema_version: 1,
    origin: $origin,
    checked_at: $checked_at,
    checks: {
      web_ready: {
        status: ($web_ready | tonumber),
        actual_release_sha: $actual_release,
        expected_release_sha: (if $expected_release == "" then null else $expected_release end),
        release_match: (if $expected_release == "" then null else $actual_release == $expected_release end)
      },
      api_health: {status: ($api_health | tonumber), contract_major: 1, shared_cache: false}
    }
  }' > "${report}"

echo "Canonical-domain smoke passed; redacted status report written to ${report}."
