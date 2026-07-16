#!/usr/bin/env python3
"""Enforce the exact, expiring non-shipped Tauri/GLib advisory exception."""

from __future__ import annotations

import datetime as dt
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
EXCEPTION_ID = "FRAME-DEP-2026-01"
ADVISORY_ID = "RUSTSEC-2024-0429"
EXPIRES = dt.date(2026, 10, 15)
SHIPPED_TARGETS = ("aarch64-apple-darwin", "x86_64-pc-windows-msvc")


def main() -> int:
    today = dt.datetime.now(dt.timezone.utc).date()
    if today >= EXPIRES:
        raise SystemExit(
            f"{EXCEPTION_ID} expired on {EXPIRES.isoformat()}; remove or re-review it"
        )
    deny = (ROOT / "deny.toml").read_text(encoding="utf-8")
    policy = (ROOT / "docs" / "security" / "dependency-policy.md").read_text(
        encoding="utf-8"
    )
    for required in (EXCEPTION_ID, ADVISORY_ID, EXPIRES.isoformat()):
        if required not in deny + policy:
            raise SystemExit(f"desktop advisory exception lost required marker {required}")

    for target in SHIPPED_TARGETS:
        command = [
            "cargo",
            "tree",
            "--locked",
            "--edges",
            "normal,build",
            "--target",
            target,
            "-p",
            "frame-desktop-core",
            "--features",
            "tauri-app",
        ]
        result = subprocess.run(
            command,
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        if "glib v0.18.5" in result.stdout:
            raise SystemExit(
                f"{ADVISORY_ID} became reachable from shipped target {target}"
            )
    print(
        f"{EXCEPTION_ID} is unexpired and unreachable from {len(SHIPPED_TARGETS)} shipped targets"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
