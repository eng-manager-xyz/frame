#!/usr/bin/env python3
"""Cross-platform, credential-free checks used by `scripts/frame doctor`."""

from __future__ import annotations

import argparse
import shutil
import socket
import sys


BROWSERS = (
    "chromium",
    "chromium-browser",
    "google-chrome",
    "msedge",
    "safaridriver",
)


def port_available(port: int) -> bool:
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
            listener.bind(("127.0.0.1", port))
    except OSError:
        return False
    return True


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ports", type=int, nargs="+", required=True)
    args = parser.parse_args()
    failed = False
    for port in args.ports:
        if not 1 <= port <= 65535:
            print(f"invalid local port: {port}", file=sys.stderr)
            failed = True
        elif port_available(port):
            print(f"local port {port}: available")
        else:
            print(
                f"local port {port}: already in use; stop the conflicting service or configure another port",
                file=sys.stderr,
            )
            failed = True

    browser = next((name for name in BROWSERS if shutil.which(name)), None)
    if browser is None:
        print(
            "no supported browser driver/runtime found (Chromium, Chrome, Edge, or safaridriver)",
            file=sys.stderr,
        )
        failed = True
    else:
        print(f"browser tooling: {browser}")

    if shutil.which("cargo-tauri") or shutil.which("tauri"):
        print("optional Tauri CLI: available")
    else:
        print("optional Tauri CLI: unavailable (required only for native shell development)")
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
