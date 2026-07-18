#!/usr/bin/env python3
"""Prove hostile GStreamer env is rejected before worker readiness initializes it."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import socket
import stat
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


LOADER_OVERRIDES = {
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "LD_AUDIT",
    "DYLD_LIBRARY_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_FRAMEWORK_PATH",
    "DYLD_FALLBACK_FRAMEWORK_PATH",
    "DYLD_ROOT_PATH",
    "DYLD_IMAGE_SUFFIX",
    "DYLD_SHARED_REGION",
}
PLUGIN_OVERRIDES = {
    "GST_PLUGIN_PATH",
    "GST_PLUGIN_PATH_1_0",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SCANNER",
    "GST_PLUGIN_SCANNER_1_0",
    "GST_REGISTRY",
    "GST_REGISTRY_1_0",
    "GST_REGISTRY_DISABLE",
    "GST_REGISTRY_UPDATE",
    "GST_REGISTRY_FORK",
    "GST_REGISTRY_MODE",
    "GST_REGISTRY_REUSE_PLUGIN_SCANNER",
    "GST_PLUGIN_LOADING_WHITELIST",
    "GST_PLUGIN_FEATURE_RANK",
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    return parser.parse_args()


def available_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def request_json(url: str) -> tuple[int, dict[str, object]]:
    try:
        with urllib.request.urlopen(url, timeout=2) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, json.load(error)


def probe_degraded(worker: Path, environment: dict[str, str]) -> tuple[int, dict[str, object]]:
    port = available_port()
    environment = environment | {"FRAME_MEDIA_ADDR": f"127.0.0.1:{port}"}
    process = subprocess.Popen(
        [str(worker), "serve"],
        env=environment,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        live_url = f"http://127.0.0.1:{port}/health/live"
        for _ in range(50):
            if process.poll() is not None:
                raise SystemExit("media worker stopped before readiness probe")
            try:
                status, _ = request_json(live_url)
                if status == 200:
                    break
            except (OSError, ValueError):
                time.sleep(0.05)
        else:
            raise SystemExit("media worker did not expose liveness")

        status, payload = request_json(f"http://127.0.0.1:{port}/health/ready")
        if status != 503:
            raise SystemExit(f"hostile GStreamer readiness returned {status}")
        if payload.get("status") != "degraded":
            raise SystemExit("hostile GStreamer readiness was not degraded")
        return status, payload
    finally:
        if process.poll() is None:
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


def main() -> None:
    args = parse_args()
    worker = args.worker.resolve(strict=True)
    plugin_root_value = os.environ.get("GST_PLUGIN_SYSTEM_PATH_1_0", "")
    plugin_root = Path(plugin_root_value).resolve(strict=True)
    candidates = sorted(plugin_root.glob("libgstvideotestsrc.*"))
    if len(candidates) != 1 or not candidates[0].is_file():
        raise SystemExit("hostile readiness test could not locate videotestsrc")

    with tempfile.TemporaryDirectory(prefix="frame-hostile-gstreamer-") as temporary:
        root = Path(temporary)
        plugin_directory = root / "plugins"
        cache_directory = root / "cache"
        plugin_directory.mkdir()
        cache_directory.mkdir()
        shutil.copy2(candidates[0], plugin_directory / candidates[0].name)
        marker = root / "scanner-invoked"
        scanner = root / "plugin-scanner"
        scanner.write_text(
            "#!/bin/sh\n" + f": > {json.dumps(str(marker))}\n" + "exit 71\n",
            encoding="utf-8",
        )
        scanner.chmod(scanner.stat().st_mode | stat.S_IXUSR)

        clean_environment = {
            name: value
            for name, value in os.environ.items()
            if name not in PLUGIN_OVERRIDES | LOADER_OVERRIDES
        }
        plugin_environment = clean_environment | {
            "GST_PLUGIN_PATH": str(plugin_directory),
            "GST_PLUGIN_SCANNER": str(scanner),
            "XDG_CACHE_HOME": str(cache_directory),
        }
        plugin_status, _ = probe_degraded(worker, plugin_environment)
        if marker.exists():
            raise SystemExit("hostile GStreamer scanner was invoked")

        loader_environment = clean_environment | {
            "LD_LIBRARY_PATH": str(plugin_directory),
            "XDG_CACHE_HOME": str(cache_directory),
        }
        loader_status, _ = probe_degraded(worker, loader_environment)
        evidence = {
            "schema_version": 2,
            "plugin_override_http_status": plugin_status,
            "loader_override_http_status": loader_status,
            "worker_status": "degraded",
            "hostile_scanner_invoked": False,
            "result": "pass",
        }
        if args.evidence.exists() and args.evidence.is_symlink():
            raise SystemExit("refusing symlinked hostile-readiness evidence")
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    print("Hostile GStreamer readiness short-circuit passed")


if __name__ == "__main__":
    main()
