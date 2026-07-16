#!/usr/bin/env python3
"""Mutation tests for the release-authority policy checker."""

from __future__ import annotations

import os
import pathlib
import shutil
import subprocess
import sys
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts" / "ci" / "check-workflow-policy.py"


def run(root: pathlib.Path) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    environment["FRAME_WORKFLOW_POLICY_ROOT"] = str(root)
    return subprocess.run(
        [sys.executable, "-I", str(CHECKER)],
        cwd=ROOT,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
        timeout=20,
    )


def replace(path: pathlib.Path, old: str, new: str) -> None:
    text = path.read_text(encoding="utf-8")
    if old not in text:
        raise AssertionError(f"mutation seed missing from {path.name}: {old!r}")
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-workflow-policy-") as temporary:
        fixture = pathlib.Path(temporary)
        shutil.copytree(ROOT / ".github", fixture / ".github")
        (fixture / ".cargo").mkdir(parents=True)
        shutil.copy2(ROOT / ".cargo/config.toml", fixture / ".cargo/config.toml")
        (fixture / "scripts/ci").mkdir(parents=True)
        shutil.copy2(
            ROOT / "scripts/ci/gstreamer-sanitized-exec",
            fixture / "scripts/ci/gstreamer-sanitized-exec",
        )
        shutil.copy2(
            ROOT / "scripts/ci/release-change-plan.sh",
            fixture / "scripts/ci/release-change-plan.sh",
        )

        baseline = run(fixture)
        if baseline.returncode != 0:
            print(baseline.stderr, file=sys.stderr)
            return 1

        production = fixture / ".github/workflows/production-gate.yml"
        share = fixture / ".github/workflows/share-player.yml"
        quality = fixture / ".github/workflows/quality-gates.yml"
        mutations = (
            (
                production,
                "  push:\n    branches: [main]",
                "  push:\n    branches: [main]\n    paths: [apps/control-plane/**]",
                "path-filtered sentinel",
            ),
            (
                production,
                "  production-gate:\n    name: production-gate\n    if: ${{ always() }}",
                "  production-gate:\n    name: production-gate\n    if: ${{ success() }}",
                "skippable sentinel",
            ),
            (
                production,
                "  workflow_dispatch:",
                "  workflow_run:\n    workflows: [Quality gates]\n    types: [completed]\n  workflow_dispatch:",
                "delayed sentinel",
            ),
            (
                production,
                "      - name: Resolve every release phase to a binary result",
                "      - name: Resolve every release phase to a binary result\n        continue-on-error: true",
                "advisory sentinel",
            ),
            (
                share,
                "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5",
                "actions/checkout@v4",
                "mutable action pin",
            ),
            (
                quality,
                "auth-d1-conformance.py",
                "auth-d1-advisory.py",
                "missing auth D1 conformance",
            ),
            (
                quality,
                "r2-storage-conformance.py",
                "r2-storage-advisory.py",
                "missing local R2 conformance",
            ),
        )
        for path, old, new, label in mutations:
            original = path.read_text(encoding="utf-8")
            replace(path, old, new)
            result = run(fixture)
            path.write_text(original, encoding="utf-8")
            if result.returncode == 0:
                print(f"workflow policy accepted {label}", file=sys.stderr)
                return 1

    print("workflow release-authority mutation suite rejected 7 unsafe designs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
