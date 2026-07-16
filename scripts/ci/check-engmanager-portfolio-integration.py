#!/usr/bin/env python3
"""Validate the pinned, static-only EngManager portfolio integration patch."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
FIXTURE_DIR = ROOT / "fixtures" / "engmanager-portfolio" / "v1"
MANIFEST_PATH = FIXTURE_DIR / "static-integration.json"
EXPECTED_BASE = "1de52bc8f25793dea3697e67765d53785c05cdfa"
EXPECTED_REPOSITORY = "https://github.com/matthewharwood/engmanager.xyz.git"
EXPECTED_FILES = {
    "_docs/frame-integration.md",
    "website/css/src/homepage.css",
    "website/src/components/nav/mod.rs",
    "website/src/config.rs",
    "website/src/main.rs",
    "website/src/pages/articles.rs",
    "website/src/pages/homepage.rs",
    "website/src/pages/search.rs",
    "website/src/router.rs",
    "website/src/sitemap.rs",
}


def fail(message: str) -> None:
    raise SystemExit(f"engmanager portfolio fixture: {message}")


def git(checkout: Path, *arguments: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", "-C", str(checkout), *arguments],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def validate_checkout(checkout: Path, patch_path: Path) -> str:
    if not checkout.is_dir():
        fail(f"checkout does not exist: {checkout}")

    head = git(checkout, "rev-parse", "HEAD")
    if head.returncode != 0:
        fail(f"not a Git checkout: {checkout}")
    if head.stdout.decode().strip() != EXPECTED_BASE:
        fail("checkout HEAD does not match the pinned portfolio commit")

    status = git(checkout, "status", "--porcelain=v1", "-z", "--untracked-files=all")
    if status.returncode != 0:
        fail(status.stderr.decode().strip() or "could not inspect checkout status")
    records = [record for record in status.stdout.decode().split("\0") if record]
    changed = {record[3:] for record in records}

    if not records:
        check = git(checkout, "apply", "--check", str(patch_path))
        if check.returncode != 0:
            fail(check.stderr.decode().strip() or "patch does not apply to pinned checkout")
        return "clean pinned checkout; patch applies"

    if changed != EXPECTED_FILES:
        unexpected = sorted(changed ^ EXPECTED_FILES)
        fail(f"patched checkout file set differs from fixture: {unexpected}")
    reverse = git(checkout, "apply", "--reverse", "--check", str(patch_path))
    if reverse.returncode != 0:
        fail(reverse.stderr.decode().strip() or "checkout does not exactly contain the patch")
    return "pinned checkout contains the patch"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--checkout",
        type=Path,
        help="optional clean or exactly patched engmanager.xyz checkout",
    )
    args = parser.parse_args()

    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1:
        fail("unsupported manifest schema")
    if manifest.get("upstream_repository") != EXPECTED_REPOSITORY:
        fail("unexpected upstream repository")
    if manifest.get("base_commit") != EXPECTED_BASE:
        fail("unexpected base commit")
    if manifest.get("integration_mode") != "static-link-only":
        fail("integration must remain static-link-only")
    if manifest.get("live_frame_data") is not False:
        fail("live Frame data must remain disabled in this stage")
    if manifest.get("frame_client_dependency") is not False:
        fail("the static stage must not add frame-client")
    if set(manifest.get("changed_files", [])) != EXPECTED_FILES:
        fail("manifest changed_files differs from the reviewed allowlist")

    patch_name = manifest.get("patch")
    if not isinstance(patch_name, str) or Path(patch_name).name != patch_name:
        fail("patch must be a local fixture filename")
    patch_path = FIXTURE_DIR / patch_name
    patch_bytes = patch_path.read_bytes()
    digest = hashlib.sha256(patch_bytes).hexdigest()
    if digest != manifest.get("patch_sha256"):
        fail(f"patch SHA-256 mismatch: {digest}")
    try:
        patch = patch_bytes.decode("utf-8")
    except UnicodeDecodeError as error:
        fail(f"patch is not UTF-8: {error}")

    headers = set(re.findall(r"^diff --git a/(.+) b/(.+)$", patch, re.MULTILINE))
    if any(left != right for left, right in headers):
        fail("renames are outside the static integration scope")
    touched = {left for left, _ in headers}
    if touched != EXPECTED_FILES:
        fail(f"patch file set differs from allowlist: {sorted(touched ^ EXPECTED_FILES)}")
    if any(path.endswith(("Cargo.toml", "Cargo.lock", ".gitignore")) for path in touched):
        fail("static integration must not change dependencies or lockfile policy")

    required = [
        'pub const DEFAULT_FRAME_ORIGIN: &str = "https://frame.engmanager.xyz/";',
        'const FRAME_ORIGIN_ENV_VAR: &str = "FRAME_ORIGIN";',
        'data-frame-integration="static"',
        'aria-label="Open Frame screen recorder"',
        "frame_origin().as_str()",
        "frame_origin: frame_origin().to_string()",
        "StatusCode::MISDIRECTED_REQUEST",
        'include_str!("../js/src/nav-router.js")',
        "apex sitemap must not contain a subdomain URL",
        "The initial integration is deliberately static.",
    ]
    for anchor in required:
        if anchor not in patch:
            fail(f"required contract anchor is missing: {anchor}")

    added_lines = "\n".join(
        line[1:]
        for line in patch.splitlines()
        if line.startswith("+") and not line.startswith("+++")
    )
    forbidden_live_code = [
        r"reqwest::Client(?:Builder)?",
        r"FrameClient::",
        r"tokio::spawn\(",
        r"\.send\(\)\.await",
    ]
    for pattern in forbidden_live_code:
        if re.search(pattern, added_lines):
            fail(f"live/request-path client code entered the static patch: {pattern}")

    checkout_result = "artifact-only validation"
    if args.checkout is not None:
        checkout_result = validate_checkout(args.checkout.resolve(), patch_path.resolve())

    print(
        "engmanager portfolio fixture: OK "
        f"({len(EXPECTED_FILES)} files, sha256={digest}, {checkout_result})"
    )


if __name__ == "__main__":
    main()
