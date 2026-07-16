# Dependency, license, and secret policy

Frame treats the lockfile, dependency graph, build sources, and checked-in
configuration as release inputs. The policy is fail-closed for unknown source
registries, Git dependencies, unapproved licenses, known vulnerabilities,
unsound advisories, yanked releases, wildcard version requirements, and the
credential signatures implemented by the repository scanner.

## Required checks

The supported dependency-policy tool is `cargo-deny` **0.20.2**. Run the same
commands from the repository root:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo deny --locked check advisories bans licenses sources
python3 scripts/ci/check-secrets.py
python3 scripts/ci/generate-cyclonedx.py --require-registry-checksums --output target/frame.cdx.json
```

`deny.toml` evaluates all Cargo features across the production Linux target,
the supported Apple and Windows targets, and `wasm32-unknown-unknown`. The
lockfile must already be current; the check must not rewrite it.

The Python scanner has no third-party dependencies. Its default invocation
first runs built-in deliberate-positive and benign-negative fixtures, then
scans eligible source and configuration paths returned by
`git ls-files --cached`. It does not print the matched value: findings contain
only the path, line, rule identifier, and a truncated SHA-256 fingerprint.

## Advisory gate

The RustSec advisory database is the configured vulnerability authority.
Vulnerabilities and security notices use cargo-deny's fail-closed behavior;
unsound advisories fail at every dependency depth, yanked versions fail, and
unmaintained advisories fail when they affect a direct workspace dependency.
There are no ignored advisories.

The advisory check requires network access to fetch or refresh the external
database. A successful license/bans/sources run does not imply that advisories
were checked. Offline execution is useful for diagnosis only and must use an
already-present database no more than seven days old; it is not release
evidence when the normal online advisory lane was skipped or unavailable.
Database outages fail the gate and require a recorded rerun after service is
restored.

## License gate

Production, development, and build dependencies are in scope. The allowed SPDX
set is deliberately permissive-license-only and is encoded in `deny.toml`.
An expression passes only when cargo-deny can satisfy it from that allowlist.
Unknown, unlicensed, or low-confidence inferred terms fail. Frame workspace
components are excluded from the third-party license decision only when their
package manifest explicitly declares `publish = false`; that exclusion does
not apply to anything they depend on. A publishable Frame package therefore
requires an explicit repository licensing decision and SPDX metadata.

Cargo metadata is the machine-readable license inventory. `cargo deny list`
may be used during review, but it is not a substitute for the enforcement
command. This gate also does not replace distribution-specific legal review or
a notice bundle. Every release handoff includes the deterministic CycloneDX
1.6 `frame.cdx.json`; package verification checks its digest and required
shape. The SBOM is a dependency inventory, not a legal approval or
vulnerability scan.

## Ban and source gate

- Wildcard dependency requirements are denied. The only exception is a local
  path dependency from a package explicitly marked `publish = false`; those
  links are resolved together from this lockfile and cannot be published to
  crates.io. Multiple-version findings are warnings because platform and
  framework graphs currently contain reviewed parallel major versions; every
  new warning still requires review in the dependency-change pull request.
- OpenSSL/native-tls and libgit2 bindings are denied. Frame's HTTP clients use
  the explicit rustls stack, and Git automation must not silently add a native
  library to shipped binaries.
- crates.io is the only allowed registry. Unknown registries and every Git
  dependency fail. If a Git source is ever approved, it must be an exact
  revision and added as the narrowest possible `allow-git` entry.

Path dependencies inside this workspace remain allowed. Vendored or private
registries are not implicitly trusted.

## Secret gate

`scripts/ci/check-secrets.py` scans UTF-8 tracked source/configuration (including
Rust, SQL, Terraform, workflow YAML, JSON, shell, Python, Ruby, JavaScript,
TypeScript, TOML, lockfiles, and environment-style config). Eligible files over
2 MB or eligible binary/non-UTF-8 files fail instead of being silently skipped.
The rules cover private-key headers, high-confidence provider token formats,
credentials embedded in URLs, and long literals assigned to secret-bearing
configuration names. Known fake/redacted fixture markers are narrowly ignored;
the provider-specific rules still take precedence.

This deterministic local gate intentionally does not claim to scan Git history,
provider-side secret stores, untracked files, binary media, build output, or
deployed environments. Provider secret scanning and repository-history review
remain complementary controls. Never weaken a rule merely to make a real
credential pass.

To test scanner behavior without creating a credential-like file:

```sh
python3 scripts/ci/check-secrets.py --self-test-only
```

The self-test constructs deliberate secret values at runtime so those values
are not themselves stored in the repository. It asserts that all deliberate
fixtures are detected and all benign reference/redaction fixtures are clean.

## Exceptions and response

There are no dependency or advisory exceptions at baseline. A proposed
exception must be narrow to an exact crate/advisory/source, include a plain-
language reason in `deny.toml`, and link to a tracked owner, approval date,
expiry date, exposure analysis, and removal issue. Expired, unused, or
unreasoned exceptions fail review. Broad organization, registry, license, or
version-range exemptions are not acceptable.

If a secret is found, stop distribution, revoke/rotate it at the authority,
inspect use and audit logs, remove it from the working tree, and determine
whether coordinated history rewriting and downstream cache/artifact cleanup are
required. Deleting the current line alone does not invalidate a leaked
credential or remove it from history.

Dependency updates must include the intentional `Cargo.toml` and `Cargo.lock`
diff, the four cargo-deny checks, the secret scan, normal workspace tests, and a
review of new duplicate-version warnings and license/source changes.
