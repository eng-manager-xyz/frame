#!/usr/bin/env bash
set -euo pipefail

if (($# == 0)); then
    targets=(.)
else
    targets=("$@")
fi

for target in "${targets[@]}"; do
    if [[ ! -e "$target" ]]; then
        printf 'error: audit target does not exist: %s\n' "$target" >&2
        exit 2
    fi
done

if ! command -v rg >/dev/null 2>&1; then
    printf 'error: ripgrep (rg) is required for the Leptos legacy audit\n' >&2
    exit 2
fi

printf 'Auditing Leptos legacy API candidates under:'
printf ' %s' "${targets[@]}"
printf '\n'

set +e
rg \
    --line-number \
    --column \
    --color=always \
    --hidden \
    --multiline \
    --pcre2 \
    --glob '*.rs' \
    --glob 'Cargo.toml' \
    --glob '!**/.git/**' \
    --glob '!**/.agents/**' \
    --glob '!**/target/**' \
    --glob '!**/node_modules/**' \
    --glob '!**/dist/**' \
    --glob '!**/vendor/**' \
    -e '\bcreate_(?:signal|rw_signal|trigger|memo|owning_memo|selector(?:_with_fn)?|effect|render_effect|action|node_ref|isomorphic_effect|resource|local_resource|multi_action|server_action|server_multi_action)\b' \
    -e '\bstore_value\b' \
    -e '\bMaybeSignal\b' \
    -e '\.(?:input_local|value_local)\s*\(' \
    -e '\bcreate_query_signal(?:_with_options)?\b' \
    -e '\b(?:NoCustomError|WrapError|ViaError)\b' \
    -e 'server_fn_error!' \
    -e 'ServerFnError\s*::\s*WrappedServerError' \
    -e '\bleptos\s*::\s*ssr\s*::\s*render_to_string\b' \
    -e '\bhandle_server_fns(?:_with_context)?\b' \
    -e 'experimental-islands' \
    -e '\bcx\s*:\s*(?:leptos(?:::prelude)?::)?Scope\b' \
    -e 'view!\s*\{\s*cx\s*,' \
    -e '\bprovide_context\s*\(\s*cx\s*,' \
    -e '\b(?:use_context|expect_context)\s*(?:::\s*<[^;\n()]+>)?\s*\(\s*cx\s*\)' \
    -e '\bexpect_context\s*::\s*<\s*(?:leptos(?:::prelude)?::)?Scope\s*>\s*\(\s*\)' \
    -e '\bcx\s*\.\s*children\s*\(' \
    -e '\b(?:mount_to_body|mount_to|hydrate_from|hydrate_body)\s*\(\s*cx\s*,' \
    -e '<For\b[^>]*\bview\s*=' \
    -e '(?s:#\s*\[\s*server\s*\([^]]*\bencoding\s*=)' \
    -- "${targets[@]}"
rg_status=$?
set -e

case "$rg_status" in
    0)
        printf '\nLegacy Leptos API candidates found.\n' >&2
        printf 'Review every match with references/migration-ledger.md.\n' >&2
        printf 'The exact-version compiler with deprecations denied remains authoritative.\n' >&2
        exit 1
        ;;
    1)
        printf 'No known legacy Leptos API candidates found.\n'
        ;;
    *)
        printf 'error: ripgrep failed with status %s\n' "$rg_status" >&2
        exit "$rg_status"
        ;;
esac
