#!/usr/bin/env python3
"""Launch the production Tauri binary and require a WebView-to-Rust marker."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MARKER = "FRAME_DESKTOP_SMOKE_V1 protocol=1 backend_truth=true"


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(64 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def main() -> int:
    default_binary = ROOT / "target" / "release" / (
        "frame-desktop.exe" if os.name == "nt" else "frame-desktop"
    )
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=default_binary)
    parser.add_argument("--timeout", type=float, default=30.0)
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()

    binary = args.binary.resolve()
    if not binary.is_file():
        raise SystemExit(f"desktop shell smoke failed: missing binary {binary}")
    environment = os.environ.copy()
    environment["FRAME_DESKTOP_SMOKE"] = "1"
    started = time.monotonic()
    process = subprocess.Popen(
        [str(binary)],
        cwd=ROOT,
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        stdout, stderr = process.communicate(timeout=args.timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, stderr = process.communicate(timeout=5)
        raise SystemExit(
            "desktop shell smoke failed: WebView never invoked the allowlisted command\n"
            + stdout[-2_000:]
            + "\n"
            + stderr[-2_000:]
        )
    elapsed_ms = round((time.monotonic() - started) * 1_000)
    if process.returncode != 0 or MARKER not in stdout.splitlines():
        raise SystemExit(
            f"desktop shell smoke failed: exit={process.returncode}, marker absent\n"
            + stdout[-2_000:]
            + "\n"
            + stderr[-2_000:]
        )

    evidence = {
        "schema_version": 1,
        "evidence_class": "production_csp_webview_smoke",
        "platform": os.environ.get("RUNNER_OS", sys.platform),
        "binary_sha256": digest(binary),
        "elapsed_ms": elapsed_ms,
        "marker": MARKER,
        "exit_code": process.returncode,
    }
    if args.evidence:
        output = args.evidence.resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    print(f"desktop production-CSP WebView smoke passed in {elapsed_ms} ms")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
