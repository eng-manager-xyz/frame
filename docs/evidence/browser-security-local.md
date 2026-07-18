# Browser security local evidence

Local deterministic evidence covers the host-only cookie value object,
session/CSRF/origin/fetch-site state machine, OAuth PKCE/audience/redirect/
replay contract, bounded return parser, server header policy, production
feature-off disposition, message validation, and privacy-safe CSP sanitizer.

```text
python3 -I scripts/ci/check-browser-security.py
cargo test --locked -p frame-domain identity
cargo test --locked -p frame-application identity
cargo test --locked -p frame-web browser_security
cargo test --locked -p frame-web return_paths_reject_open_redirects_and_query_secrets
cargo test --locked -p frame-web embed_messages_are_exact_origin_parent_scoped_and_replay_safe
cargo test --locked -p frame-web headers_deny_framing_and_capture_by_default
cargo test --locked -p frame-web embed_headers_allow_only_configured_exact_ancestors
```

The CSP tests seed URL, referrer, source-file, script-sample, and secret values;
only enum classes survive, batches stop at eight, and malformed JSON is ignored.
The static gate also rejects any Render `AuthenticatedSsr` registration or
bearer/tenant-header transport while that capability remains disabled.

Protected evidence remains: real portfolio navigation/cookie isolation,
session transport adapter and sibling-origin penetration test, OAuth provider
or optional handoff, browser CORS if enabled, parent iframe implementation,
supported-browser CSP/Permissions Policy behavior, named assistive technology,
CSP telemetry triage, cache traces, and log/referrer scan. Optional features
remain disabled until their exact matrix is collected.
