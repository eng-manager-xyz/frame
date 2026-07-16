---
title: "Migrate authentication, sessions, OTP, API keys, and account recovery to Rust/D1"
labels:
  - "phase:p2"
  - "area:d1"
  - "area:security"
  - "area:api"
  - "type:migration"
  - "risk:high"
depends_on: [12]
size: epic
---

# 13 · Migrate authentication, sessions, OTP, API keys, and account recovery to Rust/D1

## Outcome

Users and clients authenticate through Rust/D1 with equivalent security, revocation, recovery, and device behavior.

## Current Cap reference

Cap stores users, accounts, sessions, verification tokens, auth API keys, provider data, and session-version state in MySQL and integrates web, desktop, mobile, extension, OTP/email, and external identity flows.

Reference snapshot: `CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b`.

## Dependencies

[#12](./12-p2-d1-repositories-query-conformance.md)

## Scope

Port sign-up/sign-in, OAuth/account linking as approved, OTP/magic links, secure sessions/cookies, desktop/device tokens, API keys, email verification, recovery, logout/revocation, session-version invalidation, abuse controls, and audit events.

### Out of scope

Organization authorization is issue 14; billing identity and enterprise SSO beyond charter parity may be separate follow-ups.

## Deliverables

- [ ] Threat model and protocol diagrams for browser, desktop, mobile, extension, and developer API clients.
- [ ] D1 repositories and Rust services for identity, verification, sessions, keys, and revocation.
- [ ] Hardened cookie/token settings, key rotation, hashing, expiry, replay prevention, and rate limits.
- [ ] Compatibility strategy for existing sessions and staged forced reauthentication.
- [ ] Recovery and operator runbooks that do not expose verification secrets.

## Acceptance criteria

- [ ] Authentication success, invalid/expired/replayed token, account linking, logout-all, recovery, and revoked-key cases pass contract tests.
- [ ] Session fixation, CSRF, open redirect, enumeration, brute-force, token leakage, and privilege-escalation tests pass.
- [ ] Raw tokens, OTPs, OAuth secrets, cookies, and API keys never appear in D1 plaintext where hashing is applicable or in logs.
- [ ] Supported clients can transition without an unexplained logout, or a deliberate forced-login plan is approved and communicated.
- [ ] Every auth decision emits a privacy-safe audit event with correlation and reason code.

## Required test evidence

- Security test and threat-model review.
- Cross-client compatibility fixtures.
- Key rotation and session revocation rehearsal.

## Risks and open questions

- Identity migration can lock out users or preserve compromised sessions.
- D1 write contention and email-provider retries require idempotent verification flows.

## Rollout and rollback

Canary by test tenant/client, preserve legacy validation during an overlap window, and retain a kill switch that routes authentication back without accepting conflicting writes.

Before closing, attach links to implementation changes, test artifacts, operational documentation, and any ADR or parity-matrix update produced by this issue.
