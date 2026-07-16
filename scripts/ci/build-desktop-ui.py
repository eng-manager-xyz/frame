#!/usr/bin/env python3
"""Build the desktop UI from a clean tree with deterministic Trunk flags."""

from __future__ import annotations

import os
import pathlib
import subprocess


ROOT = pathlib.Path(__file__).resolve().parents[2]
DESKTOP = ROOT / "apps" / "desktop"
TRUNK_CONFIG = "ui/Trunk.toml"


def run(*args: str) -> None:
    environment = os.environ.copy()
    # Trunk 0.21 expects a boolean here, while the conventional NO_COLOR value
    # inherited by many shells is "1". Normalize it for every supported host.
    environment["NO_COLOR"] = "false"
    subprocess.run(args, cwd=DESKTOP, env=environment, check=True)


def main() -> int:
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


if __name__ == "__main__":
    raise SystemExit(main())
