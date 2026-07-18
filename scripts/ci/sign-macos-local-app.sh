#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
APP_BUNDLE=${FRAME_MACOS_APP_PATH:-"$ROOT/target/release/bundle/macos/Frame.app"}
EXPECTED_IDENTIFIER=xyz.engmanager.frame

usage() {
  cat <<'EOF'
Usage: scripts/ci/sign-macos-local-app.sh [sign|verify|verify-trusted|identities]

  sign        sign the local Frame.app, then verify its bundle identity
  verify      verify the existing bundle without changing it
  verify-trusted
              additionally require a certificate-backed Apple team identity
  identities  list installed macOS code-signing identities

Set FRAME_CODESIGN_IDENTITY to an Apple Development or Developer ID identity.
When it is unset, signing falls back to an identifier-stable ad-hoc signature;
current macOS releases can still reject that fallback for ScreenCaptureKit.
EOF
}

require_macos() {
  if [[ $(uname -s) != Darwin ]]; then
    printf 'Frame macOS app signing requires macOS\n' >&2
    exit 2
  fi
}

require_bundle() {
  if [[ ! -d $APP_BUNDLE ]]; then
    printf 'missing Frame application bundle: %s\n' "$APP_BUNDLE" >&2
    exit 2
  fi
  local plist="$APP_BUNDLE/Contents/Info.plist"
  if [[ ! -f $plist ]]; then
    printf 'missing application Info.plist: %s\n' "$plist" >&2
    exit 2
  fi
  local plist_identifier
  plist_identifier=$(plutil -extract CFBundleIdentifier raw -o - "$plist")
  if [[ $plist_identifier != "$EXPECTED_IDENTIFIER" ]]; then
    printf 'refusing to sign bundle id %s; expected %s\n' \
      "$plist_identifier" "$EXPECTED_IDENTIFIER" >&2
    exit 2
  fi
}

signature_details() {
  codesign --display --verbose=4 "$APP_BUNDLE" 2>&1
}

verify_bundle() {
  local require_trusted=${1:-0}
  require_bundle
  codesign --verify --deep --strict --verbose=2 "$APP_BUNDLE"

  local details requirement
  details=$(signature_details)
  requirement=$(codesign --display --requirements - "$APP_BUNDLE" 2>&1)
  if ! grep -Fqx "Identifier=$EXPECTED_IDENTIFIER" <<<"$details"; then
    printf 'signed application identifier does not match %s\n' \
      "$EXPECTED_IDENTIFIER" >&2
    printf '%s\n' "$details" >&2
    exit 1
  fi
  if ! grep -Fq "identifier \"$EXPECTED_IDENTIFIER\"" <<<"$requirement"; then
    printf 'designated requirement is not anchored to %s\n' \
      "$EXPECTED_IDENTIFIER" >&2
    printf '%s\n' "$requirement" >&2
    exit 1
  fi
  if [[ $require_trusted == 1 ]]; then
    local team test_requirement stable_prefix team_clause requirement_line
    team=$(sed -n 's/^TeamIdentifier=//p' <<<"$details")
    test_requirement="anchor apple generic and identifier \"$EXPECTED_IDENTIFIER\" and certificate leaf[subject.OU] = \"$team\""
    stable_prefix="designated => identifier \"$EXPECTED_IDENTIFIER\" and anchor apple generic"
    team_clause="certificate leaf[subject.OU] = \"$team\""
    requirement_line=$(sed -n '/^designated =>/p' <<<"$requirement")
    if grep -Fq "Signature=adhoc" <<<"$details" || \
      grep -Fq "TeamIdentifier=not set" <<<"$details" || \
      [[ -z $team ]] || \
      [[ $requirement_line != "$stable_prefix"* ]] || \
      [[ $requirement_line != *"$team_clause"* ]]; then
      printf '%s\n' \
        'certificate-backed Apple signing is required for protected hardware evidence' >&2
      printf '%s\n%s\n' "$details" "$requirement" >&2
      exit 1
    fi
    codesign --verify --deep --strict --verbose=2 \
      -R="$test_requirement" "$APP_BUNDLE"
  fi

  printf '%s\n' "$details" | sed -n '1,12p'
  printf '%s\n' "$requirement" | sed -n '1,3p'
}

sign_bundle() {
  require_bundle
  local identity=${FRAME_CODESIGN_IDENTITY:--}
  if [[ $identity == - ]]; then
    printf 'Signing Frame.app ad hoc with stable identifier %s\n' \
      "$EXPECTED_IDENTIFIER"
    printf '%s\n' \
      'WARNING: macOS 15+ can reject ad-hoc ScreenCaptureKit identities.' \
      'Set FRAME_CODESIGN_IDENTITY to an Apple Development or Developer ID identity for a reliable physical capture test.'
    codesign --force --deep --sign - \
      --identifier "$EXPECTED_IDENTIFIER" \
      --requirements '=designated => identifier "xyz.engmanager.frame"' \
      "$APP_BUNDLE"
  else
    printf 'Signing Frame.app as %s with identity %s\n' \
      "$EXPECTED_IDENTIFIER" "$identity"
    codesign --force --deep --sign "$identity" \
      --identifier "$EXPECTED_IDENTIFIER" \
      "$APP_BUNDLE"
  fi
  if [[ $identity == - ]]; then
    verify_bundle 0
  else
    verify_bundle 1
  fi
}

require_macos
case ${1:-sign} in
  sign)
    sign_bundle
    ;;
  verify)
    verify_bundle 0
    ;;
  verify-trusted)
    verify_bundle 1
    ;;
  identities)
    security find-identity -v -p codesigning
    ;;
  help|-h|--help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
