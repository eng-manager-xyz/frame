# Leptos modernity rubric

Apply this rubric to every migration, review, and substantial Leptos
implementation. Hard gates are mandatory. The scorecard distinguishes a merely
compiling patch from a high-quality, maintainable migration.

## Contents

- [Hard gates](#hard-gates)
- [Twelve-point scorecard](#twelve-point-scorecard)
- [Per-change checklist](#per-change-checklist)
- [Semantic review questions](#semantic-review-questions)
- [Evidence record](#evidence-record)
- [Completion rule](#completion-rule)

## Hard gates

A single failure blocks completion.

### G1. Exact compatibility contract

- Record the pinned `leptos` and companion-crate versions.
- Record enabled features, target triple, and valid `csr`/`hydrate`/`ssr`/island
  modes.
- Distinguish preserve-pin work from an explicit upgrade.

### G2. Primary-source provenance

- Support changed or uncertain API claims with exact-version docs.rs, crate
  source, crates.io, official release notes, or the current Leptos book.
- Do not use search snippets, old blog posts, model memory, or a workspace tag as
  the sole API authority.

### G3. No deprecated or removed production API

- The legacy audit has no unexplained match in changed production code.
- A compiler/check for each affected target succeeds with deprecations denied.
- No compatibility shim reintroduces a removed API under a local name.

Generated/vendor code and historical documentation may contain a legacy token
only when clearly out of production scope and reported as an exception.

### G4. Complete legacy disposition

- Inventory every legacy occurrence in the task scope.
- Map each to a verified replacement, a deliberate architectural removal, or an
  explicit no-replacement decision.
- For contextual mappings, document the preserved behavior instead of claiming a
  mechanical rename.

### G5. Target-version examples and code

- New code and documentation examples type-check against the exact target
  release or use signatures directly verified in that release.
- Prerelease-only APIs never enter stable-target examples.
- Companion APIs use the actual companion version, not the `leptos` patch number.

### G6. Independent mode validation

- Native SSR and Wasm CSR/hydration are checked separately when affected.
- SSR-plus-hydration behavior is exercised when initial markup, mounting,
  resources, suspense, or routing changes.
- A successful build in one mode is never presented as proof for another.

### G7. Architecture and repository contract preserved

- Existing ownership of routing, authentication, response state, hydration
  boundaries, assets, and privileged commands remains intact unless the task
  explicitly changes it.
- No router, server function, integration, island mode, full-body hydration, or
  dependency is introduced incidentally.

### G8. Skill ownership remains non-overlapping

- `rust-modern` remains the authority for Rust/Cargo/error/test mechanics.
- `pragmatic-tiger` remains the authority for scope, risk, performance,
  dependencies, invariants, and debt.
- This skill's decision is Leptos-specific and does not override those rules.

### G9. Claims stay within evidence

- Do not claim "latest," "faster," "smaller," "SSR-safe," "hydration-safe," or
  "drop-in" without current evidence appropriate to the claim.
- State unvalidated modes and behavior explicitly.

## Twelve-point scorecard

Score each category 0, 1, or 2.

### 1. API accuracy

- **0:** Uses an unavailable/deprecated API, wrong signature, or materially wrong
  semantics.
- **1:** Compiles on the main target but has an unresolved signature/semantic
  assumption or relies on non-primary guidance.
- **2:** Exact-version signatures and semantics are verified; no legacy API
  remains in scope.

### 2. Release coverage

- **0:** Skips an intervening migration boundary or treats versions/tags as
  interchangeable.
- **1:** Identifies source/target lines but leaves a relevant intervening change
  implicit.
- **2:** Reviews every intervening release-line transition, companion version,
  yanked/prerelease issue, and relevant patch-level facility.

### 3. Replacement actionability

- **0:** Gives only a denylist or vague "update to latest" direction.
- **1:** Names replacements but omits one important ownership, async, error, or
  rendering caveat.
- **2:** Every legacy item has a verified replacement/removal and a behavior-
  preserving review note.

### 4. Rendering and reactive correctness

- **0:** Confuses CSR/hydration/SSR, creates a hydration mismatch, misuses effects,
  or breaks local/`Send` boundaries.
- **1:** Main semantics are correct but one valid mode or async boundary is not
  exercised.
- **2:** Reactive roles, ownership/storage, async boundaries, valid HTML, initial
  markup, mounting, and all affected modes are verified.

### 5. Scope and progressive disclosure

- **0:** Introduces unrelated architecture/dependencies or duplicates sibling
  policy.
- **1:** Patch is scoped, but guidance or code contains avoidable unrelated
  complexity.
- **2:** Change is minimal, repository-shaped, and loads/changes only the relevant
  Leptos surface while deferring sibling concerns correctly.

### 6. Maintainability and evidence

- **0:** No reproducible scan/check record or leaves unexplained legacy matches.
- **1:** Core checks pass, but evidence or future-update cues are incomplete.
- **2:** Commands/results, version/features/modes, mappings, exceptions, sources,
  and profile/ledger update triggers are explicit and reproducible.

## Per-change checklist

Use this compact checklist during implementation:

- [ ] Exact version, companion versions, features, target, and mode identified.
- [ ] Preferred API exists and is non-deprecated in the exact target.
- [ ] Legacy scan is clear or every match has a documented disposition.
- [ ] Signal/memo/effect/resource/action choice matches semantic role.
- [ ] Arena/`Arc*` and sync/local storage match lifetime and threading.
- [ ] Async work has the correct SSR and serialization behavior.
- [ ] Components, children, events, control flow, and list keys use current APIs.
- [ ] SSR/hydration boundary emits valid, deterministic matching HTML.
- [ ] Router/server-function/integration changes preserve route and security
  ownership.
- [ ] Repository architecture and dependency policy remain satisfied.
- [ ] Exact affected targets compile with warnings/deprecations denied.
- [ ] Tests exercise the changed behavior, not only compilation.
- [ ] Sources and unvalidated items are reported.

## Semantic review questions

### Reactivity

- Is this mutable state, derived state, external synchronization, reactive load,
  or explicit dispatch?
- Could a cheap closure replace a memo? Could a resource replace an effect-driven
  fetch? Could an action replace a manually managed pending signal?
- Does any arena handle escape its owner? Is a local primitive accessed only on
  its originating thread?

### Views

- Does conditional type unification preserve types without unnecessary erasure?
- Are optional props passed according to stripping semantics?
- Are list keys stable identities?
- Does targeted event access match the concrete element?

### SSR and hydration

- Can the server render the same initial branch without browser-only data?
- Will the browser normalize any invalid markup before hydration?
- Does async content use the integration rendering path it requires?
- Does each `hydrate_from` root correspond exactly to server-rendered HTML, and
  does each `mount_to` root intentionally start client-only?

### Full stack

- Who owns HTTP routes, auth, context, headers, redirects, errors, and body limits?
- Are server-function inputs treated as untrusted and custom errors encoded on
  both sides?
- Is WebSocket/lazy/islands behavior an explicit requirement rather than novelty?

## Evidence record

Include a short record in the handoff or review:

```text
Leptos contract: leptos X.Y.Z; companion crates ...; features ...; targets ...
Modes checked: ssr / hydrate / csr / islands (list only actual modes)
Legacy dispositions: old API -> verified replacement/removal (one per class)
Primary sources: exact-version links or source paths
Commands: audit, format, checks, tests, build/smoke commands and results
Exceptions/unvalidated: explicit list, or "none"
Rubric: gates G1-G9 pass; score N/12
```

## Completion rule

A change passes only when:

1. all nine hard gates pass;
2. no scorecard category is zero;
3. the total is at least **10/12**; and
4. no task-required validation remains outstanding.

Do not inflate a score to compensate for a failed gate. If a target cannot be
run, report the limitation and keep the corresponding gate open unless equivalent
repository-approved evidence exists.
