#!/usr/bin/env python3
"""Validate public contract fixtures without printing potentially sensitive values."""

from __future__ import annotations

import json
import pathlib
import sys
from typing import Any
from urllib.parse import urlsplit


ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "fixtures" / "frame-api" / "v1"

FORBIDDEN_KEYS = {
    "authorization",
    "cookie",
    "email",
    "internal_cause",
    "object_key",
    "owner_email",
    "owner_id",
    "raw_body",
    "session",
    "signed_url",
    "tenant_id",
    "token",
}
FORBIDDEN_VALUE_MARKERS = (
    "x-amz-credential=",
    "x-amz-signature=",
    "bearer ",
    "-----begin private key-----",
    "frame-recordings/",
    "r2.cloudflarestorage.com",
)
ALLOWED_URL_HOSTS = {"frame.engmanager.xyz", "cdn.engmanager.xyz"}


def walk(value: Any, path: str, errors: list[str]) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            normalized = key.lower().replace("-", "_")
            if normalized in FORBIDDEN_KEYS:
                errors.append(f"{path}: forbidden public key {key!r}")
            walk(child, f"{path}.{key}", errors)
    elif isinstance(value, list):
        for index, child in enumerate(value):
            walk(child, f"{path}[{index}]", errors)
    elif isinstance(value, str):
        lowered = value.lower()
        if any(marker in lowered for marker in FORBIDDEN_VALUE_MARKERS):
            errors.append(f"{path}: contains a secret-bearing or internal value")
        if value.startswith(("http://", "https://", "//")):
            parsed = urlsplit(value)
            if parsed.scheme != "https" or parsed.hostname not in ALLOWED_URL_HOSTS:
                errors.append(f"{path}: URL is outside an approved HTTPS public host")
            if parsed.username or parsed.password or parsed.query or parsed.fragment:
                errors.append(f"{path}: URL contains credentials, query, or fragment")


def main() -> int:
    if not FIXTURES.is_dir():
        print("missing fixtures/frame-api/v1", file=sys.stderr)
        return 1

    files = sorted(FIXTURES.glob("*.json"))
    if not files:
        print("no public API fixtures found", file=sys.stderr)
        return 1

    errors: list[str] = []
    for fixture in files:
        try:
            payload = json.loads(fixture.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"{fixture.relative_to(ROOT)}: invalid JSON ({type(error).__name__})")
            continue
        walk(payload, str(fixture.relative_to(ROOT)), errors)

    if errors:
        print("public fixture validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(f"validated {len(files)} privacy-safe public contract fixtures")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
