#!/usr/bin/env python3
"""Deterministically reject high-confidence secrets in tracked source/config.

This scanner deliberately uses a small, reviewable rule set instead of entropy
guessing. It scans the checked-out content of paths reported by ``git ls-files``
and never prints a matched value. Run it with no arguments to execute both its
fixture self-test and the repository scan.
"""

from __future__ import annotations

import argparse
import hashlib
import os
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
from typing import Iterable


MAX_FILE_BYTES = 2_000_000

SOURCE_SUFFIXES = frozenset(
    {
        ".bash",
        ".c",
        ".cc",
        ".cfg",
        ".cjs",
        ".conf",
        ".cpp",
        ".csv",
        ".css",
        ".env",
        ".go",
        ".gql",
        ".graphql",
        ".h",
        ".html",
        ".ini",
        ".js",
        ".json",
        ".jsonc",
        ".jsx",
        ".key",
        ".kt",
        ".lock",
        ".md",
        ".mjs",
        ".pem",
        ".properties",
        ".proto",
        ".py",
        ".rb",
        ".rs",
        ".scss",
        ".sh",
        ".sql",
        ".swift",
        ".tf",
        ".tfvars",
        ".toml",
        ".ts",
        ".tsx",
        ".tsv",
        ".txt",
        ".xml",
        ".yaml",
        ".yml",
    }
)

SOURCE_NAMES = frozenset(
    {
        ".dev.vars",
        ".dockerignore",
        ".editorconfig",
        ".env",
        ".gitattributes",
        ".gitignore",
        ".npmrc",
        ".pypirc",
        "Cargo.lock",
        "Dockerfile",
        "Gemfile",
        "Makefile",
    }
)


@dataclass(frozen=True)
class Rule:
    identifier: str
    expression: re.Pattern[str]
    value_group: str | int = 0


@dataclass(frozen=True, order=True)
class Finding:
    path: str
    line: int
    rule: str
    fingerprint: str


@dataclass(frozen=True)
class Fixture:
    name: str
    content: str
    expected_rules: frozenset[str]


RULES = (
    Rule(
        "private-key",
        re.compile(r"-----BEGIN (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----"),
    ),
    Rule(
        "github-classic-token",
        re.compile(r"\bgh[pousr]_[A-Za-z0-9]{30,255}\b"),
    ),
    Rule(
        "github-fine-grained-token",
        re.compile(r"\bgithub_pat_[A-Za-z0-9_]{50,255}\b"),
    ),
    Rule(
        "aws-access-key-id",
        re.compile(
            r"\b(?:AKIA|ASIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA)[A-Z0-9]{16}\b"
        ),
    ),
    Rule(
        "google-api-key",
        re.compile(r"\bAIza[0-9A-Za-z_-]{35}\b"),
    ),
    Rule(
        "slack-token",
        re.compile(r"\bxox[baprs]-[0-9A-Za-z-]{20,255}\b"),
    ),
    Rule(
        "stripe-live-secret",
        re.compile(r"\bsk_live_[0-9A-Za-z]{20,255}\b"),
    ),
    Rule(
        "gitlab-access-token",
        re.compile(r"\bglpat-[0-9A-Za-z_-]{20,255}\b"),
    ),
    Rule(
        "npm-access-token",
        re.compile(r"\bnpm_[0-9A-Za-z]{36}\b"),
    ),
    Rule(
        "credential-in-url",
        re.compile(
            r"\b(?:https?|mysql|postgres(?:ql)?|redis)://"
            r"[^\s/:@]{1,128}:(?P<credential>[^\s@]{8,512})@",
            re.IGNORECASE,
        ),
        "credential",
    ),
)

LITERAL_SECRET = re.compile(
    r"""
    \b(?:
        password|passwd|pwd|secret|
        access[_-]?token|auth[_-]?token|api[_-]?token|
        api[_-]?key|client[_-]?secret|private[_-]?key|
        aws[_-]?secret[_-]?access[_-]?key|
        cloudflare[_-]?(?:api[_-]?)?token|database[_-]?url|
        github[_-]?token|gitlab[_-]?token|npm[_-]?token|
        render[_-]?api[_-]?key|slack[_-]?token|stripe[_-]?secret[_-]?key
    )\b
    \s*(?::|=|=>)\s*
    (?P<quote>["']?)
    (?P<value>[A-Za-z0-9+/_.:@-]{20,512})
    (?P=quote)
    """,
    re.IGNORECASE | re.VERBOSE,
)

SAFE_LITERAL_MARKERS = (
    "change-me",
    "changeme",
    "do-not-log",
    "dummy",
    "example",
    "fake",
    "fixture",
    "not-a-real",
    "private-session-token",
    "redacted",
    "replace-me",
    "same-secret",
    "sample",
    "super-secret",
    "test-only",
)


def fingerprint(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()[:12]


def line_number(content: str, offset: int) -> int:
    return content.count("\n", 0, offset) + 1


def is_safe_literal(value: str) -> bool:
    lowered = value.lower()
    if any(marker in lowered for marker in SAFE_LITERAL_MARKERS):
        return True
    # Repeated-character examples are placeholders, not useful credentials.
    return len(set(value)) <= 2


def scan_text(path: str, content: str) -> list[Finding]:
    findings: set[Finding] = set()

    for rule in RULES:
        for match in rule.expression.finditer(content):
            value = match.group(rule.value_group)
            findings.add(
                Finding(
                    path=path,
                    line=line_number(content, match.start()),
                    rule=rule.identifier,
                    fingerprint=fingerprint(value),
                )
            )

    for match in LITERAL_SECRET.finditer(content):
        value = match.group("value")
        if is_safe_literal(value):
            continue
        findings.add(
            Finding(
                path=path,
                line=line_number(content, match.start()),
                rule="literal-secret",
                fingerprint=fingerprint(value),
            )
        )

    return sorted(findings)


def fixtures() -> tuple[Fixture, ...]:
    github_token = "".join(("gh", "p_", "aB7" * 12))
    private_key = "".join(
        ("-" * 5, "BEGIN ", "OPENSSH PRIVATE KEY", "-" * 5)
    )
    cloudflare_token = "".join(
        ("Z7u2k9_", "Qp4m8-Vc6", "x1Ns3b5W", "d0r2Tf9H", "y4j6La8E")
    )

    return (
        Fixture(
            "deliberate-github-token",
            github_token,
            frozenset({"github-classic-token"}),
        ),
        Fixture(
            "deliberate-private-key",
            private_key,
            frozenset({"private-key"}),
        ),
        Fixture(
            "deliberate-unprefixed-provider-token",
            f"CLOUDFLARE_API_TOKEN={cloudflare_token}",
            frozenset({"literal-secret"}),
        ),
        Fixture(
            "safe-secret-references",
            "\n".join(
                (
                    "CLOUDFLARE_API_TOKEN=${CLOUDFLARE_API_TOKEN}",
                    "token: ${{ secrets.FRAME_INTERNAL_API_TOKEN }}",
                    "password = \"change-me-in-a-secret-store\"",
                )
            ),
            frozenset(),
        ),
        Fixture(
            "safe-redaction-test-values",
            "\n".join(
                (
                    'let secret = "frame-secret-do-not-log";',
                    'diagnostic_token = "fixture-diagnostic-token-value"',
                    'password = "test-only-password-value"',
                )
            ),
            frozenset(),
        ),
    )


def run_self_test() -> list[str]:
    errors: list[str] = []
    for fixture in fixtures():
        findings = scan_text(f"<fixture:{fixture.name}>", fixture.content)
        actual = frozenset(finding.rule for finding in findings)
        if actual != fixture.expected_rules:
            errors.append(
                f"fixture {fixture.name!r}: expected {sorted(fixture.expected_rules)}, "
                f"found {sorted(actual)}"
            )
    return errors


def repository_root() -> Path:
    completed = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return Path(completed.stdout.decode("utf-8").strip())


def tracked_paths(root: Path) -> list[str]:
    completed = subprocess.run(
        ["git", "-C", os.fspath(root), "ls-files", "--cached", "-z"],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    raw_paths = completed.stdout.split(b"\0")
    paths = [raw.decode("utf-8") for raw in raw_paths if raw]
    return sorted(paths)


def is_source_or_config(path: str) -> bool:
    pure = PurePosixPath(path)
    name = pure.name
    if name in SOURCE_NAMES or pure.suffix.lower() in SOURCE_SUFFIXES:
        return True
    if name.startswith((".dev.vars.", ".env.", "Dockerfile.")):
        return True
    if name.endswith((".tfvars.example", ".tfvars.json")):
        return True
    return bool(pure.parts and pure.parts[0] == "scripts")


def read_tracked_text(root: Path, relative: str) -> tuple[str | None, str | None]:
    path = root / relative
    try:
        if path.is_symlink():
            return os.readlink(path), None
        data = path.read_bytes()
    except OSError as error:
        return None, f"{relative}: cannot read tracked path: {error}"

    if len(data) > MAX_FILE_BYTES:
        return (
            None,
            f"{relative}: eligible file is {len(data)} bytes; maximum is "
            f"{MAX_FILE_BYTES} (split it or review the scanner limit)",
        )
    if b"\0" in data:
        return None, f"{relative}: eligible tracked path is binary"
    try:
        return data.decode("utf-8"), None
    except UnicodeDecodeError as error:
        return None, f"{relative}: eligible tracked path is not UTF-8: {error}"


def scan_repository(root: Path) -> tuple[list[Finding], list[str], int]:
    findings: list[Finding] = []
    errors: list[str] = []
    scanned = 0

    for relative in tracked_paths(root):
        if not is_source_or_config(relative):
            continue
        content, error = read_tracked_text(root, relative)
        if error is not None:
            errors.append(error)
            continue
        assert content is not None
        scanned += 1
        findings.extend(scan_text(relative, content))

    return sorted(findings), sorted(errors), scanned


def print_errors(prefix: str, errors: Iterable[str]) -> None:
    for error in errors:
        print(f"{prefix}: {error}", file=sys.stderr)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--self-test-only",
        action="store_true",
        help="run the built-in deliberate and benign fixtures without scanning Git",
    )
    mode.add_argument(
        "--scan-only",
        action="store_true",
        help="scan tracked source/config without running fixture self-tests",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()

    if not args.scan_only:
        fixture_errors = run_self_test()
        if fixture_errors:
            print_errors("secret scanner self-test failed", fixture_errors)
            return 1
        print(f"secret scanner self-test passed ({len(fixtures())} fixtures)")

    if args.self_test_only:
        return 0

    try:
        root = repository_root()
        findings, scan_errors, scanned = scan_repository(root)
    except (OSError, subprocess.CalledProcessError, UnicodeDecodeError) as error:
        print(f"secret scan failed: cannot enumerate tracked paths: {error}", file=sys.stderr)
        return 1

    print_errors("secret scan failed", scan_errors)
    for finding in findings:
        print(
            f"secret scan failed: {finding.path}:{finding.line}: {finding.rule} "
            f"(sha256:{finding.fingerprint})",
            file=sys.stderr,
        )

    if scan_errors or findings:
        return 1

    print(f"secret scan passed ({scanned} tracked source/config files)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
