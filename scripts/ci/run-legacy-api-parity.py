#!/usr/bin/env python3
"""Run the complete local legacy/API parity contract in a fixed order.

The inventory is intentionally discovered from the committed
``legacy-*-sqlite-conformance.py`` files so adding a new legacy surface cannot
silently omit it from pull-request, local, or production-preflight coverage.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shlex
import subprocess
import sys


ROOT = pathlib.Path(
    os.environ.get(
        "FRAME_LEGACY_API_PARITY_ROOT", pathlib.Path(__file__).resolve().parents[2]
    )
).resolve()
CI = ROOT / "scripts" / "ci"

# Preserve the privacy-safe evidence filenames uploaded by
# api-workflow-parity.yml. Suites not listed here still run, but retain their
# existing stdout-only behavior.
EVIDENCE_FILES = {
    "api-workflow-d1-conformance.py": "api-workflow-d1-conformance.json",
    "legacy-api-execution-sqlite-conformance.py": (
        "legacy-api-execution-sqlite-conformance.json"
    ),
    "legacy-collaboration-sqlite-conformance.py": (
        "legacy-collaboration-sqlite-conformance.json"
    ),
    "legacy-developer-actions-sqlite-conformance.py": (
        "legacy-developer-actions-sqlite-conformance.json"
    ),
    "legacy-folder-assignment-sqlite-conformance.py": (
        "legacy-folder-assignment-sqlite-conformance.json"
    ),
    "legacy-folder-crud-sqlite-conformance.py": (
        "legacy-folder-crud-sqlite-conformance.json"
    ),
    "legacy-library-placement-sqlite-conformance.py": (
        "legacy-library-placement-sqlite-conformance.json"
    ),
    "legacy-membership-actions-sqlite-conformance.py": (
        "legacy-membership-actions-sqlite-conformance.json"
    ),
    "legacy-notification-actions-sqlite-conformance.py": (
        "legacy-notification-actions-sqlite-conformance.json"
    ),
    "legacy-user-account-sqlite-conformance.py": (
        "legacy-user-account-sqlite-conformance.json"
    ),
    "legacy-video-properties-sqlite-conformance.py": (
        "legacy-video-properties-sqlite-conformance.json"
    ),
}

LEGACY_SCRIPT_NAMES = (
    "legacy-analytics-sqlite-conformance.py",
    "legacy-api-execution-sqlite-conformance.py",
    "legacy-collaboration-sqlite-conformance.py",
    "legacy-core-storage-sqlite-conformance.py",
    "legacy-desktop-compatibility-sqlite-conformance.py",
    "legacy-desktop-session-sqlite-conformance.py",
    "legacy-developer-actions-sqlite-conformance.py",
    "legacy-developer-api-sqlite-conformance.py",
    "legacy-extension-auth-sqlite-conformance.py",
    "legacy-extension-instant-recordings-sqlite-conformance.py",
    "legacy-folder-assignment-sqlite-conformance.py",
    "legacy-folder-crud-sqlite-conformance.py",
    "legacy-invite-lifecycle-sqlite-conformance.py",
    "legacy-library-detail-reads-sqlite-conformance.py",
    "legacy-library-id-reads-sqlite-conformance.py",
    "legacy-library-placement-sqlite-conformance.py",
    "legacy-membership-actions-sqlite-conformance.py",
    "legacy-mobile-bootstrap-caps-sqlite-conformance.py",
    "legacy-mobile-session-sqlite-conformance.py",
    "legacy-mobile-uploads-sqlite-conformance.py",
    "legacy-notification-actions-sqlite-conformance.py",
    "legacy-notification-preferences-sqlite-conformance.py",
    "legacy-notification-read-sqlite-conformance.py",
    "legacy-org-custom-domain-sqlite-conformance.py",
    "legacy-organization-library-sqlite-conformance.py",
    "legacy-organization-selection-sqlite-conformance.py",
    "legacy-protected-billing-auth-sqlite-conformance.py",
    "legacy-protected-integrations-sqlite-conformance.py",
    "legacy-protected-media-sqlite-conformance.py",
    "legacy-space-authorization-sqlite-conformance.py",
    "legacy-transcripts-sqlite-conformance.py",
    "legacy-upload-storage-sqlite-conformance.py",
    "legacy-user-account-sqlite-conformance.py",
    "legacy-video-domain-info-sqlite-conformance.py",
    "legacy-video-lifecycle-sqlite-conformance.py",
    "legacy-video-properties-sqlite-conformance.py",
)


class ParityRunnerError(RuntimeError):
    """Raised when the committed runner inventory is incomplete."""


def script_inventory() -> list[pathlib.Path]:
    fixed = [
        CI / "check-api-workflow-parity.py",
        CI / "check-migrations.py",
        CI / "api-workflow-d1-conformance.py",
    ]
    committed_legacy = {
        path.name for path in CI.glob("legacy-*-sqlite-conformance.py")
    }
    declared_legacy = set(LEGACY_SCRIPT_NAMES)
    if committed_legacy != declared_legacy:
        undeclared = sorted(committed_legacy - declared_legacy)
        missing = sorted(declared_legacy - committed_legacy)
        details = []
        if undeclared:
            details.append(f"undeclared committed scripts: {', '.join(undeclared)}")
        if missing:
            details.append(f"declared missing scripts: {', '.join(missing)}")
        raise ParityRunnerError("; ".join(details))
    legacy = [CI / name for name in LEGACY_SCRIPT_NAMES]
    trailing = [CI / "compatibility-rate-limit-sqlite-conformance.py"]
    scripts = [*fixed, *legacy, *trailing]
    missing = [path for path in scripts if not path.is_file()]
    if missing:
        rendered = ", ".join(str(path.relative_to(ROOT)) for path in missing)
        raise ParityRunnerError(f"missing parity runner input: {rendered}")
    if len({path.name for path in scripts}) != len(scripts):
        raise ParityRunnerError("parity runner inventory contains duplicate script names")
    return scripts


def commands(evidence_dir: pathlib.Path) -> list[list[str]]:
    result: list[list[str]] = []
    for script in script_inventory():
        command = [sys.executable, "-I", str(script.relative_to(ROOT))]
        evidence_name = EVIDENCE_FILES.get(script.name)
        if evidence_name is not None:
            command.extend(["--evidence", str(evidence_dir / evidence_name)])
        result.append(command)
    return result


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--evidence-dir",
        type=pathlib.Path,
        default=pathlib.Path("target/evidence"),
        help="directory for the workflow-retained privacy-safe evidence files",
    )
    parser.add_argument(
        "--list-json",
        action="store_true",
        help="print the exact command inventory without executing it",
    )
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        planned = commands(args.evidence_dir)
    except ParityRunnerError as error:
        print(f"legacy/API parity runner failed: {error}", file=sys.stderr)
        return 1

    if args.list_json:
        print(json.dumps(planned, indent=2))
        return 0

    total = len(planned)
    for index, command in enumerate(planned, start=1):
        print(
            f"legacy/API parity [{index}/{total}]: {shlex.join(command)}",
            flush=True,
        )
        try:
            completed = subprocess.run(command, cwd=ROOT, check=False)
        except OSError as error:
            print(
                f"legacy/API parity runner could not start {command[2]}: {error}",
                file=sys.stderr,
            )
            return 1
        if completed.returncode != 0:
            print(
                f"legacy/API parity runner stopped at {command[2]} "
                f"with status {completed.returncode}",
                file=sys.stderr,
            )
            return completed.returncode

    print(f"legacy/API parity runner passed all {total} deterministic commands")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
