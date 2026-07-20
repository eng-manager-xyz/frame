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
    printf 'error: ripgrep (rg) is required for the Axum legacy audit\n' >&2
    exit 2
fi

common_args=(
    --line-number
    --column
    --color=always
    --hidden
    --multiline
    --pcre2
    --glob '*.rs'
    --glob 'Cargo.toml'
    --glob '!**/.git/**'
    --glob '!**/.agents/**'
    --glob '!**/target/**'
    --glob '!**/node_modules/**'
    --glob '!**/dist/**'
    --glob '!**/vendor/**'
    --glob '!**/generated/**'
    --glob '!**/snapshots/**'
)

printf 'Auditing Axum legacy API candidates under:'
printf ' %s' "${targets[@]}"
printf '\n\nHard legacy candidates:\n'

set +e
rg "${common_args[@]}" \
    -e '\baxum\s*::\s*prelude\b' \
    -e '\bRoutingDsl\b' \
    -e '\baxum\s*::\s*route\s*\(' \
    -e '\baxum\s*::\s*handler\s*::\s*(?:get|post|put|patch|delete|head|options|trace)\b' \
    -e '\bRouter\s*::\s*or\s*\(' \
    -e '\brouting\s*::\s*(?:handler_method_router|service_method_router|service_method_routing)\b' \
    -e '\b(?:AddExtensionLayer|UrlParamsMap?|PathParamsRejection)\b' \
    -e '\b(?:ContentLengthLimit|RequestParts|BodyAlreadyExtracted)\b' \
    -e '\bextractor_middleware\b' \
    -e '#\s*\[\s*axum\s*::\s*async_trait\s*\]' \
    -e '\b(?:use\s+)?axum\s*::\s*async_trait\b' \
    -e '\b(?:Router|MethodRouter|FromRequest|FromRequestParts)\s*<[^>\n]*,\s*(?:B|Body|ReqBody|RequestBody)\b' \
    -e '\bHandler\s*<[^,>\n]+,[^,>\n]+,\s*(?:B|Body|ReqBody|RequestBody)\b' \
    -e '\bNext\s*<' \
    -e '\bOption\s*<\s*Query\s*<' \
    -e '\baxum\s*::\s*extract\s*::\s*(?:RawBody|BodyStream)\b' \
    -e '(?s)\buse\s+axum\s*::\s*extract\s*::\s*\{[^}]{0,500}\b(?:RawBody|BodyStream)\b' \
    -e '\baxum\s*::\s*body\s*::\s*(?:BoxBody|box_body|boxed|Full|Empty)\b' \
    -e '(?s)\buse\s+axum\s*::\s*body\s*::\s*\{[^}]{0,500}\b(?:BoxBody|box_body|boxed|Full|Empty)\b' \
    -e '\bbody\s*::\s*BoxBody\b' \
    -e '\bbody\s*::\s*(?:box_body|boxed)\s*\(' \
    -e '\bhyper\s*::\s*Body\b' \
    -e '\bhyper\s*::\s*body\s*::\s*to_bytes\b' \
    -e '\b(?:axum|hyper)\s*::\s*Server\b' \
    -e '\bServer\s*::\s*bind\s*\(' \
    -e '\baxum\s*::\s*(?:TypedHeader|headers)\b' \
    -e '(?s)\buse\s+axum\s*::\s*\{[^;]{0,1000}\b(?:TypedHeader|headers)\b[^;]*;' \
    -e '\baxum\s*::\s*extract\s*::\s*Host\b' \
    -e '(?s)\buse\s+axum\s*::\s*extract\s*::\s*\{[^;]{0,500}\bHost\b[^;]*;' \
    -e '(?s)\buse\s+axum\s*::\s*\{[^;]{0,1000}\bextract\s*::\s*(?:Host\b|\{[^;]{0,500}\bHost\b)[^;]*;' \
    -e '\baxum_extra\s*::\s*extract\s*::\s*(?:Host|Scheme|OptionalPath)\b' \
    -e '(?s)\buse\s+axum_extra\s*::\s*extract\s*::\s*\{[^;]{0,500}\b(?:Host|Scheme|OptionalPath)\b[^;]*;' \
    -e '(?s)\buse\s+axum_extra\s*::\s*\{[^;]{0,1000}\bextract\s*::\s*(?:Host\b|\{[^;]{0,500}\b(?:Host|Scheme|OptionalPath)\b)[^;]*;' \
    -e '\bWebSocketUpgrade\s*::\s*max_send_queue\b' \
    -e '\.\s*max_send_queue\s*\(' \
    -e '\baxum\s*::\s*sse\b' \
    -e '\bRedirect\s*::\s*found\s*\(' \
    -e '\b(?:DefaultOnFailedUpdgrade|OnFailedUpdgrade)\b' \
    -e '\b(?:RouterService|check_infallible|inherit_state)\b' \
    -e '(?s)impl[^{};]*\bIntoResponse\b[^{}]*\{[^{}]*\btype\s+(?:Body|BodyError)\b' \
    -e '\b(?:poll_data|poll_trailers)\s*\(' \
    -e '\bhttp_body\s*::\s*combinators\b' \
    -e '\b(?:ready_and|ReadyAnd)\b' \
    -e '\btower\s*::\s*util\s*::\s*Either\s*::\s*(?:A|B)\b' \
    -e '\btower_http\s*::\s*timeout\s*::\s*TimeoutLayer\s*::\s*new\s*\(' \
    -e '\btower_http\s*::\s*timeout\s*::\s*Timeout\s*::\s*(?:new|layer)\s*\(' \
    -e '\btower_http\s*::\s*auth\s*::\s*require_authorization\b' \
    -e '\btower_http\s*::\s*cors\s*::\s*any\s*\(' \
    -e '\baxum-debug\b' \
    -- "${targets[@]}"
hard_status=$?
set -e

if ((hard_status > 1)); then
    printf 'error: hard-candidate scan failed with status %s\n' "$hard_status" >&2
    exit "$hard_status"
fi

printf '\nManual-review signals (not automatically legacy):\n'

set +e
rg "${common_args[@]}" \
    -e '(?:r\#*|br\#*|b)?"[^"\n]*(?:/:[A-Za-z_][A-Za-z0-9_]*|/\*[A-Za-z_][A-Za-z0-9_]*)' \
    -e '(?s)\bRouter\s*::\s*new\s*\(\s*\)[^;]{0,2000}\.\s*or\s*\(' \
    -e '\b(?:RawBody|BodyStream|BoxBody)\b' \
    -e '\.\s*without_v07_checks\s*\(' \
    -e '\bEither\s*::\s*(?:A|B)\b' \
    -e '\b(?:TimeoutLayer|Timeout)\s*::\s*(?:new|layer)\s*\(' \
    -e '\b(?:cors\s*::\s*any|require_authorization)\b' \
    -e '(?s)\.\s*layer\s*\(\s*Extension\s*\(' \
    -e '\bDefaultBodyLimit\s*::\s*disable\s*\(' \
    -e '(?s)\bto_bytes\s*\([^,]+,\s*usize\s*::\s*MAX\s*\)' \
    -e '\b(?:socket|websocket|ws)\s*\.\s*close\s*\(\s*\)\s*\.\s*await\b' \
    -e '\bCorsLayer\s*::\s*permissive\s*\(' \
    -e '\.\s*tcp_nodelay\s*\(' \
    -e '(?i)\b(?:x-forwarded-(?:for|host|proto)|forwarded)\b' \
    -e '(?m)^\s*(?:axum|http|http-body|hyper|hyper-util|tower|tower-http)\s*=\s*' \
    -- "${targets[@]}"
review_status=$?
set -e

if ((review_status > 1)); then
    printf 'error: review-signal scan failed with status %s\n' "$review_status" >&2
    exit "$review_status"
fi

if ((review_status == 0)); then
    printf '\nReview every signal for legitimate current semantics, version alignment, and policy.\n' >&2
else
    printf 'No manual-review signals found.\n'
fi

if ((hard_status == 0)); then
    printf '\nLegacy Axum API candidates found.\n' >&2
    printf 'Disposition every match with references/migration-ledger.md.\n' >&2
    printf 'The exact-version compiler with deprecations denied remains authoritative.\n' >&2
    exit 1
fi

printf '\nNo known hard legacy Axum API candidates found.\n'
