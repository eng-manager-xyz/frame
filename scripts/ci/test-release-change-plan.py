#!/usr/bin/env python3
"""Exercise release impact classification for compile-time fixture paths."""

from __future__ import annotations

import pathlib
import shutil
import subprocess
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[2]
PLANNER = ROOT / "scripts" / "ci" / "release-change-plan.sh"


def git(root: pathlib.Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", *arguments],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def classify(root: pathlib.Path, before: str, after: str) -> dict[str, str]:
    result = subprocess.run(
        [str(root / "release-change-plan.sh"), before, after],
        cwd=root,
        check=True,
        capture_output=True,
        text=True,
    )
    return dict(
        line.split("=", maxsplit=1)
        for line in result.stdout.splitlines()
        if "=" in line and not line.startswith("reason=")
    )


def main() -> int:
    cases = (
        ("fixtures/api-parity/v1/route-workflow-report.json", True, False),
        ("fixtures/api-parity/v1/changelog-feed.json", True, False),
        ("fixtures/web-authenticated/v1/route-matrix.json", False, True),
        ("fixtures/web-authenticated/v1/browser-direct-boundary.json", False, True),
        ("crates/authenticated-client/src/lib.rs", True, True),
        ("docs/notes.md", False, False),
    )
    with tempfile.TemporaryDirectory(prefix="frame-release-plan-") as directory:
        fixture = pathlib.Path(directory)
        shutil.copy2(PLANNER, fixture / "release-change-plan.sh")
        git(fixture, "init", "--quiet")
        git(fixture, "config", "user.email", "release-plan@frame.invalid")
        git(fixture, "config", "user.name", "Frame release plan")
        (fixture / "baseline").write_text("baseline\n", encoding="utf-8")
        git(fixture, "add", ".")
        git(fixture, "commit", "--quiet", "-m", "baseline")
        for index, (relative, worker, web) in enumerate(cases, start=1):
            before = git(fixture, "rev-parse", "HEAD")
            path = fixture / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"case {index}\n", encoding="utf-8")
            git(fixture, "add", relative)
            git(fixture, "commit", "--quiet", "-m", f"case {index}")
            after = git(fixture, "rev-parse", "HEAD")
            result = classify(fixture, before, after)
            if result.get("worker_changed") != str(worker).lower():
                raise AssertionError(f"Worker impact drifted for {relative}")
            if result.get("web_changed") != str(web).lower():
                raise AssertionError(f"web impact drifted for {relative}")
    print(f"release change plan classified {len(cases)} compile-time and unrelated path cases")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
