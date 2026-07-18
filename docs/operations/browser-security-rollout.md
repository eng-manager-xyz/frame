# Browser capability rollout and rollback

## Launch state

- enable only top-level portfolio navigation;
- keep portfolio handoff, direct browser CORS, authenticated SSR, and public
  embed disabled;
- enforce host-only session/CSRF and deny framing/capture by default;
- run the privacy-safe CSP collector, group by release/directive/resource
  class, and alert only on a bounded sustained increase.

Before promotion, inspect reports in staging and report-only policy without
widening to `https:`, wildcard origins, inline script, or capture permissions.
The committed policy stays enforcing. A raw CSP report is forbidden in logs,
traces, artifacts, alerts, and support bundles; retention and access follow the
operational privacy contract.

## Optional capability gates

1. Handoff: exact callback/audience, state/nonce, S256 PKCE, TTL/replay,
   provider adapter, log/referrer scan, portfolio implementation, and kill
   switch.
2. Browser CORS: one endpoint at a time, exact origin/method/header/credential
   matrix, preflight/actual/error/redirect tests, cache and abuse review.
3. Public player embed: exact parent CSP/frame-src and Frame ancestors, minimal
   iframe `allow`/sandbox, message harness, private-state matrix, supported
   browser/mobile/accessibility evidence, and consent review.

Recorder embedding is not a rollout option; capture stays top-level.

## Incident rollback

Disable the affected capability flag first. Handoff rollback stops new code
issuance and revokes pending exchanges without logging codes. CORS rollback
removes the exact origin/endpoint response path. Embed rollback disables the
Frame flag and parent iframe while retaining the ordinary share link. A CSP
collector incident disables reporting, not enforcement; purge any raw report
that escaped and follow the privacy incident runbook. Frame login and top-level
navigation remain available throughout.

Attach header captures, cookie/CSRF and redirect matrices, hostile message
results, CSP summary, browser versions, approver, timestamps, and rollback
timing. Real sibling-host cookies, providers, browsers, CSP telemetry, and
portfolio parent code are protected evidence.
