#!/usr/bin/env python3
"""Build the desktop UI from a clean tree with deterministic Trunk flags."""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
DESKTOP = ROOT / "apps" / "desktop"
TRUNK_CONFIG = "ui/Trunk.toml"
NPM = "npm.cmd" if os.name == "nt" else "npm"


def run(*args: str) -> None:
    environment = os.environ.copy()
    # Trunk 0.21 expects a boolean here, while the conventional NO_COLOR value
    # inherited by many shells is "1". Normalize it for every supported host.
    environment["NO_COLOR"] = "false"
    subprocess.run(args, cwd=DESKTOP, env=environment, check=True)


def main() -> int:
    run_at_root(NPM, "run", "build:ui-css")
    run_at_root(sys.executable, "-I", "scripts/ci/check-ui-styles.py")
    run_at_root(sys.executable, "-I", "scripts/ci/check-ui-migration.py")
    run("trunk", "clean", "--config", TRUNK_CONFIG)
    run(
        "trunk",
        "build",
        "--config",
        TRUNK_CONFIG,
        "--release",
        "--locked",
    )
    return 0


def run_at_root(*args: str) -> None:
    environment = os.environ.copy()
    environment["NO_COLOR"] = "false"
    if not (ROOT / "node_modules" / ".bin" / "tailwindcss").is_file():
        subprocess.run(
            (NPM, "ci", "--ignore-scripts"),
            cwd=ROOT,
            env=environment,
            check=True,
        )
    subprocess.run(args, cwd=ROOT, env=environment, check=True)


if __name__ == "__main__":
    raise SystemExit(main())
