#!/usr/bin/env python3
"""Evaluate the credential-free Frame cache and account-IaC contract."""

from __future__ import annotations

import json
import pathlib
import re
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
CASES = ROOT / "fixtures/cloudflare/v1/cache-security-cases.json"
ACCOUNT = ROOT / "infra/cloudflare-account"


def classify(case: dict[str, Any]) -> str:
    request = case["request"]
    response = case["response"]
    method = request["method"]
    path = request["path"]
    pathname = path.split("?", 1)[0]
    cache_control = response["cache_control"].lower()
    bypass_paths = (
        "/api",
        "/health",
        "/login",
        "/logout",
        "/signup",
        "/verify",
        "/dashboard",
        "/settings",
        "/billing",
        "/admin",
        "/imports",
    )
    sensitive_query = "?" in path and any(
        marker in path.lower() for marker in ("signature", "token", "credential")
    )
    if (
        request["host"] != "frame.engmanager.xyz"
        or method not in {"GET", "HEAD"}
        or any(pathname == prefix or pathname.startswith(prefix + "/") for prefix in bypass_paths)
        or request.get("authorization", False)
        or request.get("cookie", False)
        or request.get("range", False)
        or request.get("upgrade", False)
        or sensitive_query
        or response["set_cookie"]
        or response["privacy"] != "public"
        or any(value in cache_control for value in ("private", "no-store", "no-cache"))
    ):
        return "bypass"
    fingerprinted = re.fullmatch(
        r"/assets/[A-Za-z0-9_-]+\.[0-9a-f]{12,64}\.[A-Za-z0-9]+", pathname
    )
    if fingerprinted and "max-age=31536000" in cache_control and "immutable" in cache_control:
        return "immutable"
    return "bypass"


def main() -> int:
    errors: list[str] = []
    try:
        fixture = json.loads(CASES.read_text(encoding="utf-8"))
        if fixture.get("schema_version") != 1:
            errors.append("cache fixture schema version drifted")
        cases = fixture["cases"]
    except (OSError, json.JSONDecodeError, KeyError, TypeError) as error:
        print(f"invalid cache fixture ({type(error).__name__})", file=sys.stderr)
        return 1
    identifiers: set[str] = set()
    for case in cases:
        identifier = case.get("id")
        if not isinstance(identifier, str) or identifier in identifiers:
            errors.append("cache cases require unique string IDs")
            continue
        identifiers.add(identifier)
        actual = classify(case)
        if actual != case.get("expected"):
            errors.append(f"{identifier}: expected {case.get('expected')}, got {actual}")
    if len(cases) < 20 or sum(case.get("expected") == "immutable" for case in cases) != 2:
        errors.append("cache matrix must retain 20 cases and exactly two immutable controls")

    versions = (ACCOUNT / "versions.tf").read_text(encoding="utf-8")
    lock = (ACCOUNT / ".terraform.lock.hcl").read_text(encoding="utf-8")
    main_tf = (ACCOUNT / "main.tf").read_text(encoding="utf-8")
    variables = (ACCOUNT / "variables.tf").read_text(encoding="utf-8")
    combined = "\n".join(path.read_text(encoding="utf-8") for path in ACCOUNT.glob("*.tf"))
    if 'version = "5.21.1"' not in versions or 'version     = "5.21.1"' not in lock:
        errors.append("Cloudflare provider pin or lockfile drifted")
    for forbidden in (
        "cloudflare_record",
        "cloudflare_ruleset",
        "cloudflare_zone",
        "cloudflare_zone_setting",
        "cloudflare_worker",
    ):
        if forbidden in combined:
            errors.append(f"account state illegally owns {forbidden}")
    for required in (
        'resource "cloudflare_r2_bucket" "recordings"',
        'resource "cloudflare_r2_bucket_cors" "recordings"',
        'resource "cloudflare_r2_bucket_lifecycle" "recordings"',
        "prevent_destroy = true",
        'methods = ["GET", "HEAD", "PUT"]',
        '"content-length"',
        '"if-none-match"',
        '"x-amz-checksum-sha256"',
        '"x-amz-meta-frame-sha256"',
        'prefix = "uploads/"',
        "abort_multipart_uploads_transition",
    ):
        if required not in main_tf:
            errors.append(f"missing account-IaC invariant: {required}")
    if 'strcontains(origin, "*")' not in variables or "allowed_browser_origins" not in variables:
        errors.append("R2 CORS origin allowlist is not exact")

    if errors:
        print("Cloudflare edge/account policy failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(
        f"validated {len(cases)} cache/security cases and isolated pinned R2 account state"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
