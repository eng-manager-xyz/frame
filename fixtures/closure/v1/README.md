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
The current exact partition is 352 locally satisfied, 171 protected pending,
and 31 repository-local gaps. The focused native-product audit pins those gaps
to issue 24 checkboxes 1–10, issue 25 checkboxes 1/4/5/6/7/8, issue 27
checkboxes 2/3/4/5/8/9/10/11, and issue 33 checkboxes 3/4/5/6/8/9/10.

Run:

```sh
python3 -I scripts/ci/check-issue-closure.py
```

The checker requires an exact 554-item partition and prints every true local
gap with its issue, ordinal, and source text. Protected classes are deliberately
allowed to overlap because one checkbox can require, for example, both a real
provider and a supported browser. The inventory does not check boxes, authorize
promotion, or convert `not_collected` protected records into evidence. Its
focused ordinal assertion prevents accidental reclassification of the four
audited issue sets; it is a regression guard for the ledger, not semantic proof
that implementation exists or remains absent.
