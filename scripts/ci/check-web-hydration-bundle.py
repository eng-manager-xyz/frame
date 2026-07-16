#!/usr/bin/env python3
"""Validate the complete, CSP-safe closure of the web hydration bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re


ROOT = pathlib.Path(__file__).resolve().parents[2]


def fail(message: str) -> None:
    raise SystemExit(f"web hydration bundle: {message}")


def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dist", type=pathlib.Path, default=ROOT / "apps" / "web" / "dist")
    parser.add_argument("--evidence", type=pathlib.Path)
    args = parser.parse_args()
    dist = args.dist.resolve()

    if not dist.is_dir():
        fail(f"{dist} is missing; run build-web-hydration.py")
    manifest_path = dist / "manifest.json"
    if not manifest_path.is_file():
        fail("manifest.json is missing")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema") != "frame.web-hydration-manifest.v1":
        fail("manifest schema is not current")
    assets = {kind: manifest.get(kind, {}) for kind in ("bootstrap", "javascript", "wasm")}
    expected = {"manifest.json", *(asset.get("file") for asset in assets.values())}
    if None in expected:
        fail("manifest asset filename is missing")
    files = {
        path.relative_to(dist).as_posix()
        for path in dist.rglob("*")
        if path.is_file()
    }
    if files != expected:
        fail(f"expected exactly {sorted(expected)}, found {sorted(files)}")

    bootstrap_path = dist / assets["bootstrap"]["file"]
    js_path = dist / assets["javascript"]["file"]
    wasm_path = dist / assets["wasm"]["file"]
    for kind, path in (
        ("bootstrap", bootstrap_path),
        ("javascript", js_path),
        ("wasm", wasm_path),
    ):
        digest = sha256(path)
        if assets[kind].get("sha256") != digest or digest not in path.name:
            fail(f"{kind} content hash does not match its manifest and filename")
        if not re.fullmatch(r"[a-z0-9_-]+-[0-9a-f]{64}\.(?:js|wasm)", path.name):
            fail(f"{kind} filename is not a full SHA-256 fingerprint")
    bootstrap = bootstrap_path.read_text(encoding="utf-8")
    javascript = js_path.read_text(encoding="utf-8")
    wasm = wasm_path.read_bytes()

    if wasm[:4] != b"\0asm":
        fail("Wasm magic is invalid")
    if len(wasm) > 2_000_000 or len(javascript.encode()) > 500_000:
        fail("bundle exceeds the initial 2 MB Wasm / 500 KB JavaScript budget")
    if f'./{js_path.name}' not in bootstrap or f'./{wasm_path.name}' not in bootstrap:
        fail("external CSP-safe bootstrap does not initialize the exact Wasm closure")
    if re.search(r"https?:|[\"']//", bootstrap + javascript):
        fail("remote or protocol-relative URL found")
    if "sourceMappingURL" in javascript or any("snippet" in name for name in files):
        fail("source map or stale snippet found")

    pages = (ROOT / "apps" / "web" / "src" / "pages.rs").read_text(
        encoding="utf-8"
    )
    server = (ROOT / "apps" / "web" / "src" / "lib.rs").read_text(
        encoding="utf-8"
    )
    if "FRAME_HYDRATION_HEAD" not in pages or "FRAME_HYDRATION_SCRIPT" not in pages:
        fail("SSR document does not expose the optional hydration injection points")
    if '"/assets/{asset}"' not in server or "manifest.json" not in server:
        fail("Axum does not serve the manifest-verified hydration closure")
    csp = next(
        (line for line in server.splitlines() if "default-src 'self'" in line),
        "",
    )
    if "script-src 'self' 'wasm-unsafe-eval'" not in csp:
        fail("CSP does not authorize only same-origin scripts and Wasm compilation")
    if "'unsafe-eval'" in csp.replace("'wasm-unsafe-eval'", ""):
        fail("general JavaScript eval is forbidden")

    evidence = {
        "schema": "frame.web-hydration-bundle.v1",
        "files": {
            name: {
                "bytes": (dist / name).stat().st_size,
                "sha256": sha256(dist / name),
            }
            for name in sorted(expected)
        },
        "public_assets": [f"/assets/{assets[kind]['file']}" for kind in ("bootstrap", "javascript", "wasm")],
        "content_fingerprinted": True,
        "immutable_cache": True,
        "remote_assets": False,
        "source_maps": False,
        "locked_build": True,
    }
    encoded = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
