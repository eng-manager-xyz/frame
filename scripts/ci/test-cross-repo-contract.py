#!/usr/bin/env python3
"""Mutation tests for cross-repository contract and preview policy."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts/ci/check-cross-repo-contract.py"
FILES = (
    ".github/workflows/cross-repository-contract.yml",
    ".github/workflows/quality-gates.yml",
    ".github/workflows/production-gate.yml",
    "crates/frame-client/src/dto.rs",
    "docs/architecture/cross-repository-contract-ci.md",
    "docs/operations/cross-repository-preview.md",
    "docs/evidence/cross-repo-preview-local.md",
    "fixtures/cross-repo-preview/v1/ci-policy.json",
    "fixtures/cross-repo-preview/v1/compatibility-cases.json",
    "fixtures/cross-repo-preview/v1/contract.json",
    "fixtures/engmanager-portfolio/v1/static-integration.json",
    "scripts/ci/cross-repo-preview-e2e.py",
    "scripts/ci/verify-portfolio-consumer.py",
    "scripts/frame",
)


def run(root: Path) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    environment["FRAME_CROSS_REPO_ROOT"] = str(root)
    return subprocess.run(
        [sys.executable, "-I", str(CHECKER)],
        cwd=ROOT,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
        timeout=20,
    )


def replace(path: Path, old: str, new: str) -> None:
    text = path.read_text(encoding="utf-8")
    if old not in text:
        raise AssertionError(f"mutation seed missing from {path}: {old!r}")
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-cross-repo-policy-") as temporary:
        fixture = Path(temporary)
        for relative in FILES:
            source = ROOT / relative
            target = fixture / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        shutil.copytree(
            ROOT / "fixtures/frame-api/v1",
            fixture / "fixtures/frame-api/v1",
        )

        baseline = run(fixture)
        if baseline.returncode != 0:
            print(baseline.stderr, file=sys.stderr)
            return 1

        policy = fixture / "fixtures/cross-repo-preview/v1/ci-policy.json"
        workflow = fixture / ".github/workflows/cross-repository-contract.yml"
        cases = fixture / "fixtures/cross-repo-preview/v1/compatibility-cases.json"
        contract = fixture / "fixtures/cross-repo-preview/v1/contract.json"
        mutations = (
            (policy, '"attempts": 1', '"attempts": 2', "silent local retry"),
            (
                policy,
                '"production_secrets_allowed": false',
                '"production_secrets_allowed": true',
                "production secret in preview",
            ),
            (
                policy,
                '      "token"\n',
                '      "trace_id"\n',
                "artifact token redaction removal",
            ),
            (
                workflow,
                "permissions:\n  contents: read",
                "permissions:\n  contents: write",
                "cross-repository write authority",
            ),
            (
                workflow,
                "1de52bc8f25793dea3697e67765d53785c05cdfa",
                "1de52bc8f25793dea3697e67765d53785c05cdfb",
                "unreviewed portfolio revision",
            ),
            (
                cases,
                '"expected_current_consumer": "reject"',
                '"expected_current_consumer": "accept"',
                "breaking change accepted",
            ),
            (
                policy,
                '"frame_client_consumer": false',
                '"frame_client_consumer": true',
                "static patch promoted to consumer evidence",
            ),
            (
                policy,
                '"concurrency": 1',
                '"concurrency": 2',
                "overlapping protected canaries",
            ),
            (
                contract,
                '    "handler-path-upstream-fetch",\n    "audit-sensitive-field"\n',
                '    "handler-path-upstream-fetch"\n',
                "missing artifact-redaction control",
            ),
        )
        for path, old, new, label in mutations:
            original = path.read_text(encoding="utf-8")
            replace(path, old, new)
            result = run(fixture)
            path.write_text(original, encoding="utf-8")
            if result.returncode == 0:
                print(f"cross-repository checker accepted {label}", file=sys.stderr)
                return 1

    print("cross-repository policy mutation suite rejected 9 unsafe designs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
