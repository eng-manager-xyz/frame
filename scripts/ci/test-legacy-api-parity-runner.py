#!/usr/bin/env python3
"""Test legacy/API runner inventory, evidence paths, and fail-fast behavior."""

from __future__ import annotations

import json
import os
import pathlib
import subprocess
import sys
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[2]
RUNNER = ROOT / "scripts" / "ci" / "run-legacy-api-parity.py"


def invoke(*arguments: str, root: pathlib.Path = ROOT) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    environment["FRAME_LEGACY_API_PARITY_ROOT"] = str(root)
    return subprocess.run(
        [sys.executable, "-I", str(RUNNER), *arguments],
        cwd=ROOT,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
        timeout=20,
    )


def validate_real_inventory() -> None:
    result = invoke("--list-json", "--evidence-dir", "target/custom-evidence")
    if result.returncode != 0:
        raise AssertionError(result.stderr)
    commands = json.loads(result.stdout)
    names = [pathlib.Path(command[2]).name for command in commands]
    legacy = sorted(
        path.name
        for path in (ROOT / "scripts/ci").glob("legacy-*-sqlite-conformance.py")
    )
    expected = [
        "check-api-workflow-parity.py",
        "check-migrations.py",
        "api-workflow-d1-conformance.py",
        *legacy,
        "compatibility-rate-limit-sqlite-conformance.py",
    ]
    if names != expected or len(names) != len(set(names)):
        raise AssertionError("legacy/API runner inventory is incomplete or nondeterministic")

    evidence_names = {
        pathlib.Path(command[command.index("--evidence") + 1]).name
        for command in commands
        if "--evidence" in command
    }
    expected_evidence = {
        "api-workflow-d1-conformance.json",
        "legacy-api-execution-sqlite-conformance.json",
        "legacy-collaboration-sqlite-conformance.json",
        "legacy-developer-actions-sqlite-conformance.json",
        "legacy-folder-assignment-sqlite-conformance.json",
        "legacy-folder-crud-sqlite-conformance.json",
        "legacy-library-placement-sqlite-conformance.json",
        "legacy-membership-actions-sqlite-conformance.json",
        "legacy-notification-actions-sqlite-conformance.json",
        "legacy-user-account-sqlite-conformance.json",
        "legacy-video-properties-sqlite-conformance.json",
    }
    if evidence_names != expected_evidence:
        raise AssertionError("workflow-retained evidence filenames drifted")
    for command in commands:
        if "--evidence" in command:
            evidence = command[command.index("--evidence") + 1]
            if not evidence.startswith("target/custom-evidence/"):
                raise AssertionError("custom evidence directory was not preserved")


def validate_local_entrypoints() -> None:
    frame = (ROOT / "scripts/frame").read_text(encoding="utf-8")
    check = frame.split("  check)\n", maxsplit=1)[1].split(
        "  test)\n", maxsplit=1
    )[0]
    test = frame.split("  test)\n", maxsplit=1)[1].split(
        "  migrate)\n", maxsplit=1
    )[0]
    marker = "python3 -I scripts/ci/run-legacy-api-parity.py"
    if check.count(marker) != 1 or test.count(marker) != 1:
        raise AssertionError(
            "scripts/frame check and test must each invoke the aggregate runner exactly once"
        )


def validate_hermetic_conformance() -> None:
    offenders = []
    for script in sorted(
        (ROOT / "scripts/ci").glob("legacy-*-sqlite-conformance.py")
    ):
        if ".tmp/cap" in script.read_text(encoding="utf-8"):
            offenders.append(script.name)
    if offenders:
        raise AssertionError(
            "legacy conformance must use committed source-pin evidence, not the "
            f"discardable Cap checkout: {', '.join(offenders)}"
        )


def write_fixture_script(path: pathlib.Path, status: int) -> None:
    path.write_text(
        "import pathlib\n"
        f"pathlib.Path('trace.log').open('a', encoding='utf-8').write('{path.name}\\n')\n"
        f"raise SystemExit({status})\n",
        encoding="utf-8",
    )


def validate_fail_fast() -> None:
    with tempfile.TemporaryDirectory(prefix="frame-legacy-api-runner-") as directory:
        fixture = pathlib.Path(directory)
        ci = fixture / "scripts/ci"
        ci.mkdir(parents=True)
        write_fixture_script(ci / "check-api-workflow-parity.py", 0)
        write_fixture_script(ci / "check-migrations.py", 7)
        write_fixture_script(ci / "api-workflow-d1-conformance.py", 0)
        for source in (ROOT / "scripts/ci").glob("legacy-*-sqlite-conformance.py"):
            write_fixture_script(ci / source.name, 0)
        write_fixture_script(ci / "compatibility-rate-limit-sqlite-conformance.py", 0)

        result = invoke(root=fixture)
        if result.returncode != 7:
            raise AssertionError(
                f"runner did not preserve the first failing status: {result.returncode}"
            )
        trace = (fixture / "trace.log").read_text(encoding="utf-8").splitlines()
        if trace != ["check-api-workflow-parity.py", "check-migrations.py"]:
            raise AssertionError(f"runner did not stop at the first failure: {trace}")


def main() -> int:
    validate_real_inventory()
    validate_local_entrypoints()
    validate_hermetic_conformance()
    validate_fail_fast()
    print(
        "legacy/API parity runner inventory, hermetic evidence, and fail-fast tests passed"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
