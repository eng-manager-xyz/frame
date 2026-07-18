#!/usr/bin/env python3
"""Credential-free safety tests for the exact Frame purge tool."""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
TOOL = ROOT / "scripts/ops/frame_cache_purge.py"


def invoke(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-I", str(TOOL), *arguments],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )


def main() -> int:
    accepted = invoke("--url", "https://frame.engmanager.xyz/s/public-demo")
    if accepted.returncode != 0:
        print(accepted.stderr, file=sys.stderr)
        return 1
    payload = json.loads(accepted.stdout)
    if payload["purge"] != {"files": ["https://frame.engmanager.xyz/s/public-demo"]}:
        return 1
    tag = invoke("--tag", "frame:share:public-demo:g7")
    if tag.returncode != 0:
        return 1
    rejected = (
        ("--url", "https://engmanager.xyz/s/public-demo"),
        ("--url", "https://frame.engmanager.xyz/s/demo?token=secret"),
        ("--url", "http://frame.engmanager.xyz/s/demo"),
        ("--url", "https://user@frame.engmanager.xyz/s/demo"),
        ("--tag", "portfolio:all"),
        ("--tag", "frame:*"),
        ("--url", "https://frame.engmanager.xyz/s/demo", "--apply"),
    )
    for arguments in rejected:
        if invoke(*arguments).returncode == 0:
            print(f"unsafe purge accepted: {arguments}", file=sys.stderr)
            return 1
    print("scoped purge tool rejected 7 cross-host, credential, wildcard, and apply hazards")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
