#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

fail=0

check_absent() {
  local package=$1
  local forbidden=$2
  local tree
  tree=$(cargo tree --locked -p "$package" --edges normal --prefix none)
  if printf '%s\n' "$tree" | rg -q "$forbidden"; then
    printf 'forbidden dependency in %s matching %s\n' "$package" "$forbidden" >&2
    printf '%s\n' "$tree" >&2
    fail=1
  fi
}

check_absent frame-control-plane '^(gstreamer|glib|frame-media|frame-web|axum|leptos) v'
check_absent frame-web '^(gstreamer|glib|frame-media|worker|worker-sys|frame-control-plane) v'
check_absent frame-client '^(gstreamer|glib|frame-media|frame-domain|frame-ports|worker|worker-sys|axum|leptos) v'
check_absent frame-application '^(gstreamer|glib|frame-media|worker|worker-sys|axum|leptos) v'
check_absent frame-domain '^(gstreamer|glib|worker|worker-sys|axum|leptos|tokio) v'
check_absent frame-ports '^(gstreamer|glib|worker|worker-sys|axum|leptos) v'

if [[ $fail -ne 0 ]]; then
  exit 1
fi

cargo check --locked -p frame-domain --target wasm32-unknown-unknown
cargo check --locked -p frame-ports --target wasm32-unknown-unknown
cargo check --locked -p frame-client --no-default-features --target wasm32-unknown-unknown
cargo check --locked -p frame-application --target wasm32-unknown-unknown
cargo check --locked -p frame-control-plane --target wasm32-unknown-unknown

printf 'workspace runtime boundaries verified\n'
