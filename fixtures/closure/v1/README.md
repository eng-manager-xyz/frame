# Issue checkbox closure inventory

`checkbox-status.json` classifies every deliverable and acceptance checkbox in
issues 01–44 without modifying the issue source. The audit has three statuses:

- `local_satisfied`: code, tests, or an executable repository policy supports
  the locally decidable claim. This is never provider or production sign-off.
- `protected_pending`: the remaining claim intrinsically needs provider
  credentials, representative hardware, supported browser/assistive
  technology, production-shaped data, delivery-control settings, another
  repository, or accountable human approval.
- `local_gap`: a required implementation remains possible in Frame without
  protected access. A fake, schema, runbook, or protected blocker cannot hide
  this status.

Ordinals are one-based in checkbox order within each issue. Every issue pins a
digest of its checkbox text, so editing or reordering a requirement forces a
fresh classification. Evidence paths must be real, non-symlink repository
files. All ordinals not explicitly protected or open are locally satisfied.
The current exact partition is 371 locally satisfied, 183 protected pending,
and 0 repository-local gaps. Issue 33 checkbox 9 now has a signed macOS and
Authenticode Windows product driver, fail-closed validator, and explicit
single-display, dual mixed-scale, and rotated cells. It remains open as
protected pending until every supported-OS cell executes on representative
hardware. Issue 33 checkboxes 6 and 8 likewise have complete local
state/browser implementations and await representative native hardware and
named VoiceOver/Narrator execution. Issues 24, 25, and 27 have no
repository-local gap; their representative physical matrices remain protected
pending. Issue 27's reviewed legacy-project corpus is protected pending as
well.

Run:

```sh
python3 -I scripts/ci/check-issue-closure.py
```

The checker requires an exact 554-item partition and prints every true local
gap with its issue, ordinal, and source text. The current inventory prints
none. Protected classes are deliberately
allowed to overlap because one checkbox can require, for example, both a real
provider and a supported browser. The inventory does not check boxes, authorize
promotion, or convert `not_collected` protected records into evidence. Its
focused ordinal assertion prevents accidental reclassification of the four
audited issue sets; it is a regression guard for the ledger, not semantic proof
that implementation exists or remains absent.
