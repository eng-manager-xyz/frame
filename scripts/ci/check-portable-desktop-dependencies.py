#!/usr/bin/env python3
"""Reject native media crates from the portable Tauri dependency closure."""

from __future__ import annotations

import argparse
import re
from pathlib import Path


MAX_INPUT_BYTES = 2_000_000
ANSI_ESCAPE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
DENIED_PACKAGE = re.compile(
    r"^(?:"
    r"frame-media|"
    r"frame-platform-lifecycle|"
    r"frame-macos-screen-capture|"
    r"frame-macos-av-capture|"
    r"frame-windows-screen-capture|"
    r"frame-windows-capture-ffi|"
    r"wgc|"
    r"gstreamer\S*"
    r") v[0-9]"
)


def read_tree(path: Path) -> str:
    if path.is_symlink() or not path.is_file():
        raise SystemExit("portable dependency evidence must be a regular file")
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise SystemExit(f"could not read portable dependency evidence: {error}") from error
    if not raw or len(raw) > MAX_INPUT_BYTES or b"\x00" in raw:
        raise SystemExit("portable dependency evidence has an invalid size or encoding")
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError as error:
        raise SystemExit("portable dependency evidence must be UTF-8") from error


def denied_dependencies(tree: str) -> list[tuple[int, str]]:
    findings = []
    for line_number, raw_line in enumerate(tree.splitlines(), start=1):
        line = ANSI_ESCAPE.sub("", raw_line).strip()
        if DENIED_PACKAGE.search(line) is not None:
            findings.append((line_number, line))
    return findings


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("tree", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    findings = denied_dependencies(read_tree(args.tree))
    if findings:
        for line_number, package in findings:
            print(f"{args.tree}:{line_number}: denied native dependency: {package}")
        raise SystemExit(
            "portable desktop dependency graph contains native media code"
        )
    print("portable desktop dependency closure excludes native media code")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
