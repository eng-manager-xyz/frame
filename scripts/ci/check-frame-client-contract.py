#!/usr/bin/env python3
"""Fail closed when the Frame public client boundary or closure artifacts drift."""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import tomllib


ROOT = pathlib.Path(__file__).resolve().parents[2]
CRATE = ROOT / "crates" / "frame-client"
FORBIDDEN_SOURCE_MARKERS = (
    "frame_domain::",
    "frame_media::",
    "frame_ports::",
    "gstreamer::",
    "leptos::",
    "worker::",
    "axum::",
    "object_key",
    "signed_url",
)
REQUIRED_ARTIFACTS = (
    "docs/architecture/frame-public-contract-v1.md",
    "docs/operations/frame-client-upgrade.md",
    "docs/evidence/frame-client-local.md",
    "fixtures/frame-api/v1/contract.schema.json",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    errors: list[str] = []
    manifest = tomllib.loads((CRATE / "Cargo.toml").read_text(encoding="utf-8"))

    dependencies = manifest.get("dependencies", {})
    if set(dependencies) != {"serde", "serde_json", "url"}:
        errors.append("core dependency allowlist drifted")
    features = manifest.get("features", {})
    if features != {"default": [], "client": ["dep:reqwest"]}:
        errors.append("feature boundary drifted")
    target_dependencies = manifest.get("target", {}).get(
        "cfg(not(target_arch = \"wasm32\"))", {}
    ).get("dependencies", {})
    reqwest = target_dependencies.get("reqwest", {})
    if reqwest.get("workspace") is not True or reqwest.get("optional") is not True:
        errors.append("native reqwest must remain optional and outside wasm")

    source_files = sorted((CRATE / "src").glob("*.rs"))
    combined = "\n".join(path.read_text(encoding="utf-8").lower() for path in source_files)
    for marker in FORBIDDEN_SOURCE_MARKERS:
        if marker in combined:
            errors.append(f"forbidden public-boundary source marker: {marker}")

    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    if "crates/frame-client" not in workspace.get("workspace", {}).get("members", []):
        errors.append("frame-client is not a workspace member")

    control_plane = (ROOT / "apps/control-plane/src/lib.rs").read_text(encoding="utf-8")
    routing = (ROOT / "apps/control-plane/src/routing.rs").read_text(encoding="utf-8")
    for marker in (
        "Route::ApiHealth",
        "public_health_response(env, config)",
        "Route::PublicShare { share_id }",
        "Route::PublicMedia { share_id }",
    ):
        if marker not in control_plane:
            errors.append(f"missing Worker integration marker: {marker}")
    for path in ("/api/v1/health", "/api/v1/public/shares/"):
        if path not in routing:
            errors.append(f"missing public route: {path}")

    missing = [name for name in REQUIRED_ARTIFACTS if not (ROOT / name).is_file()]
    errors.extend(f"missing closure artifact: {name}" for name in missing)

    result = {
        "schema_version": 1,
        "status": "failed" if errors else "passed",
        "core_dependencies": sorted(dependencies),
        "native_transport": "optional_reqwest_non_wasm",
        "public_routes": [
            "GET /api/v1/health",
            "GET /api/v1/public/shares/{id}",
            "GET|HEAD|OPTIONS /api/v1/public/shares/{id}/media",
        ],
        "checked_source_files": len(source_files),
        "errors": errors,
    }
    if args.evidence:
        evidence = args.evidence if args.evidence.is_absolute() else ROOT / args.evidence
        evidence.parent.mkdir(parents=True, exist_ok=True)
        evidence.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    if errors:
        print("frame-client contract validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(
        f"validated frame-client boundary ({len(source_files)} sources, "
        f"{len(dependencies)} core dependencies, 3 public route groups)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
