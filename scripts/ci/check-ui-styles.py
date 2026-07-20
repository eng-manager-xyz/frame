#!/usr/bin/env python3
"""Verify the pinned Tailwind input produces the committed bounded stylesheet."""

from __future__ import annotations

import os
import pathlib
import subprocess
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[2]
INPUT = ROOT / "crates" / "ui" / "styles" / "tailwind.css"
OUTPUT = ROOT / "crates" / "ui" / "styles" / "tailwind.generated.css"
MAXIMUM_BYTES = 96_000
NPM = "npm.cmd" if os.name == "nt" else "npm"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-tailwind-") as directory:
        candidate = pathlib.Path(directory) / "tailwind.css"
        subprocess.run(
            (
                NPM,
                "exec",
                "--offline",
                "--",
                "tailwindcss",
                "-i",
                str(INPUT),
                "-o",
                str(candidate),
                "--minify",
            ),
            cwd=ROOT,
            check=True,
        )
        expected = OUTPUT.read_bytes()
        actual = candidate.read_bytes()

    require(actual == expected, "Tailwind output is stale; run `npm run build:ui-css`")
    require(len(actual) <= MAXIMUM_BYTES, f"Tailwind CSS exceeds {MAXIMUM_BYTES} bytes")
    require(b"@import" not in actual, "Tailwind CSS still contains an unresolved import")
    require(b"--color-background" in actual, "Tailwind CSS is missing Frame theme tokens")
    print(f"verified minified Tailwind bundle: {len(actual)} bytes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
