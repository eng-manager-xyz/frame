#!/usr/bin/env python3
"""Fail closed when the generated desktop WebView bundle is stale or unsafe."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
from pathlib import Path
from typing import NoReturn


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_DIST = ROOT / "apps" / "desktop" / "ui" / "dist"


def fail(message: str) -> NoReturn:
    raise SystemExit(f"desktop bundle check failed: {message}")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def one(paths: list[Path], label: str) -> Path:
    if len(paths) != 1:
        fail(f"expected exactly one {label}, found {len(paths)}")
    return paths[0]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dist", type=Path, default=DEFAULT_DIST)
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()

    dist = args.dist.resolve()
    if not dist.is_dir():
        fail(f"missing dist directory: {dist}")
    files = sorted(path for path in dist.rglob("*") if path.is_file())
    relative = [path.relative_to(dist).as_posix() for path in files]
    if any("snippets/" in path or path.endswith(".map") for path in relative):
        fail("stale snippet or source-map output is present")

    index = dist / "index.html"
    css = one(list(dist.glob("app-*.css")), "fingerprinted stylesheet")
    javascript = one(
        list(dist.glob("frame-desktop-ui-*.js")), "fingerprinted JavaScript loader"
    )
    wasm = one(
        list(dist.glob("frame-desktop-ui-*_bg.wasm")), "fingerprinted Wasm module"
    )
    expected = sorted(
        path.relative_to(dist).as_posix()
        for path in (index, css, javascript, wasm)
    )
    if relative != expected:
        fail(f"unexpected bundle file set: {relative}")
    if wasm.read_bytes()[:4] != b"\x00asm":
        fail("Wasm artifact has an invalid magic header")

    html = index.read_text(encoding="utf-8")
    if "http://" in html or "https://" in html or "tauri.js" in html:
        fail("index contains a remote or removed JavaScript dependency")
    for asset in (css, javascript, wasm):
        if f"/{asset.name}" not in html:
            fail(f"index does not reference {asset.name}")
    references = set(re.findall(r"['\"](/[A-Za-z0-9_./-]+)", html))
    missing = sorted(
        reference for reference in references if not (dist / reference[1:]).is_file()
    )
    if missing:
        fail(f"index references missing assets: {missing}")

    tauri = json.loads(
        (ROOT / "apps" / "desktop" / "tauri.conf.json").read_text(encoding="utf-8")
    )
    csp = tauri["app"]["security"]["csp"]
    if "script-src 'self' 'wasm-unsafe-eval'" not in csp:
        fail("production CSP does not permit only the Wasm compilation primitive")
    if "'unsafe-eval'" in csp.replace("'wasm-unsafe-eval'", ""):
        fail("production CSP enables general unsafe-eval")
    if tauri["app"].get("withGlobalTauri") is not True:
        fail("the supported Leptos bridge is disabled")
    if tauri["app"]["security"].get("capabilities") != ["frame-desktop-windows"]:
        fail("unexpected capability selection")

    capability = json.loads(
        (ROOT / "apps" / "desktop" / "capabilities" / "main.json").read_text(
            encoding="utf-8"
        )
    )
    if capability.get("windows") != ["main"]:
        fail("bootstrap capability is not restricted to the main window")
    expected_permissions = [
        "allow-bootstrap-main",
        "allow-bootstrap-desktop",
        "allow-dispatch-main",
    ]
    if capability.get("permissions") != expected_permissions:
        fail("desktop capability drifted from the three-command boundary")
    explicit_permissions = (
        ROOT / "apps" / "desktop" / "permissions" / "desktop.toml"
    ).read_text(encoding="utf-8")
    for command in ("bootstrap_desktop", "dispatch_main"):
        if f'commands.allow = ["{command}"]' not in explicit_permissions:
            fail(f"desktop permission does not isolate {command}")
    if capability.get("platforms") != ["macOS", "windows"]:
        fail("desktop platform boundary drifted")

    records = [
        {
            "path": path.relative_to(dist).as_posix(),
            "bytes": path.stat().st_size,
            "sha256": sha256(path),
        }
        for path in files
    ]
    evidence = {
        "schema_version": 1,
        "evidence_class": "deterministic_desktop_bundle",
        "platform": os.environ.get("RUNNER_OS", "local"),
        "files": records,
        "csp_sha256": hashlib.sha256(csp.encode()).hexdigest(),
        "capability_sha256": sha256(
            ROOT / "apps" / "desktop" / "capabilities" / "main.json"
        ),
    }
    if args.evidence:
        output = args.evidence.resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    print(f"desktop bundle is closed and reproducible ({len(files)} files)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
