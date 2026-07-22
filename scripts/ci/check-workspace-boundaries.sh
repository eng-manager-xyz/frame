#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

if ! command -v rg >/dev/null 2>&1; then
  printf 'workspace boundary check requires ripgrep (rg)\n' >&2
  exit 1
fi

fail=0

check_absent() {
  local package=$1
  local forbidden=$2
  local tree
  tree=$(cargo tree --locked -p "$package" --edges normal --prefix none)
  # The first line is the package under test; only dependency rows are subject
  # to the forbidden-pattern policy.
  if printf '%s\n' "$tree" | sed '1d' | rg -q "$forbidden"; then
    printf 'forbidden dependency in %s matching %s\n' "$package" "$forbidden" >&2
    printf '%s\n' "$tree" >&2
    fail=1
  fi
}

check_absent frame-control-plane '^(gstreamer|glib|frame-media|frame-web|axum|leptos) v'
check_absent frame-web '^(gstreamer|glib|frame-media|worker|worker-sys|frame-control-plane) v'
check_absent frame-client '^(gstreamer|glib|frame-media|frame-domain|frame-ports|worker|worker-sys|axum|leptos) v'
check_absent frame-authenticated-client '^(gstreamer|glib|frame-client|frame-media|frame-domain|frame-ports|worker|worker-sys|axum|leptos|reqwest) v'
check_absent frame-windows-secure-spool '^(gstreamer|glib|frame-(application|authenticated-client|client|domain|media|ports)|worker|worker-sys|axum|leptos|reqwest|tokio) v'
check_absent frame-windows-capture-ffi '^(gstreamer|glib|frame-[^[:space:]]+|worker|worker-sys|axum|leptos|reqwest|tokio) v'
check_absent frame-application '^(gstreamer|glib|frame-media|worker|worker-sys|axum|leptos) v'
check_absent frame-domain '^(gstreamer|glib|worker|worker-sys|axum|leptos|tokio) v'
check_absent frame-ports '^(gstreamer|glib|worker|worker-sys|axum|leptos) v'

if rg -n \
  '(^|[^[:alnum:]_])unsafe[[:space:]]+(fn|extern|impl|trait)|unsafe[[:space:]]*\{' \
  apps crates --glob '*.rs' --glob '!crates/windows-secure-spool/**' \
  --glob '!crates/windows-capture-ffi/**'; then
  printf 'unsafe Rust escaped the audited Windows FFI crates\n' >&2
  fail=1
fi

if rg -q 'windows-sys' crates/media/Cargo.toml crates/media/src; then
  printf 'frame-media depends directly on windows-sys instead of the safe FFI crate\n' >&2
  fail=1
fi

if [[ $fail -ne 0 ]]; then
  exit 1
fi

cargo check --locked -p frame-domain --target wasm32-unknown-unknown
cargo check --locked -p frame-ports --target wasm32-unknown-unknown
cargo check --locked -p frame-client --no-default-features --target wasm32-unknown-unknown
cargo check --locked -p frame-authenticated-client --target wasm32-unknown-unknown
cargo check --locked -p frame-application --target wasm32-unknown-unknown
cargo check --locked -p frame-control-plane --target wasm32-unknown-unknown

printf 'workspace runtime boundaries verified\n'
