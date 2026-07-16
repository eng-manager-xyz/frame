#!/usr/bin/env python3
"""Build the browser hydration module from a clean, locked Trunk input."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess


ROOT = pathlib.Path(__file__).resolve().parents[2]
WEB = ROOT / "apps" / "web"
TRUNK_CONFIG = "Trunk.toml"


def run(*args: str) -> None:
    environment = os.environ.copy()
    environment["NO_COLOR"] = "false"
    subprocess.run(args, cwd=WEB, env=environment, check=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--runtime-dir",
        type=pathlib.Path,
        help="copy the verified closure next to a production executable",
    )
    args = parser.parse_args()
    run("trunk", "clean", "--config", TRUNK_CONFIG)
    run(
        "trunk",
        "build",
        "--config",
        TRUNK_CONFIG,
        "--release",
        "--locked",
    )

    dist = WEB / "dist"
    javascript = dist / "frame-web-hydrate.js"
    wasm = dist / "frame-web-hydrate_bg.wasm"
    javascript_bytes = javascript.read_bytes()
    wasm_bytes = wasm.read_bytes()
    javascript_sha = hashlib.sha256(javascript_bytes).hexdigest()
    wasm_sha = hashlib.sha256(wasm_bytes).hexdigest()
    javascript_name = f"frame-web-hydrate-{javascript_sha}.js"
    wasm_name = f"frame-web-hydrate_bg-{wasm_sha}.wasm"
    javascript.replace(dist / javascript_name)
    wasm.replace(dist / wasm_name)

    bootstrap_bytes = (
        f'import init from "./{javascript_name}";\n\n'
        "await init({\n"
        f'  module_or_path: new URL("./{wasm_name}", import.meta.url),\n'
        "});\n"
    ).encode("utf-8")
    bootstrap_sha = hashlib.sha256(bootstrap_bytes).hexdigest()
    bootstrap_name = f"frame-web-bootstrap-{bootstrap_sha}.js"
    (dist / bootstrap_name).write_bytes(bootstrap_bytes)
    (dist / "index.html").unlink()

    manifest = {
        "schema": "frame.web-hydration-manifest.v1",
        "bootstrap": {"file": bootstrap_name, "sha256": bootstrap_sha},
        "javascript": {"file": javascript_name, "sha256": javascript_sha},
        "wasm": {"file": wasm_name, "sha256": wasm_sha},
    }
    (dist / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    if args.runtime_dir:
        runtime_dir = args.runtime_dir.resolve()
        if runtime_dir.name != "web-dist" or not runtime_dir.is_relative_to(ROOT):
            raise SystemExit(
                "--runtime-dir must be a web-dist directory inside this checkout"
            )
        if runtime_dir.exists():
            shutil.rmtree(runtime_dir)
        shutil.copytree(dist, runtime_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
