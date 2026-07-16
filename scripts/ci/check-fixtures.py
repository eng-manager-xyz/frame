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
SCHEMA = FIXTURES / "contract.schema.json"
EXPECTED_FIXTURES = {
    "error.json",
    "health.additive.json",
    "health.ok.json",
    "share.deleted.json",
    "share.failed.json",
    "share.private.json",
    "share.processing.json",
    "share.public.json",
    "share.unavailable.json",
}

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

    files = sorted(path for path in FIXTURES.glob("*.json") if path != SCHEMA)

    errors: list[str] = []
    names = {path.name for path in files}
    if names != EXPECTED_FIXTURES:
        errors.append("fixtures/frame-api/v1: canonical fixture inventory drifted")
    try:
        schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        errors.append(f"{SCHEMA.relative_to(ROOT)}: invalid JSON ({type(error).__name__})")
        schema = {}
    definitions = schema.get("$defs") if isinstance(schema, dict) else None
    if schema.get("$schema") != "https://json-schema.org/draft/2020-12/schema":
        errors.append("fixtures/frame-api/v1/contract.schema.json: schema dialect drifted")
    if not isinstance(definitions, dict) or set(definitions) != {
        "apiError",
        "apiVersion",
        "captionTrack",
        "health",
        "playbackDescriptor",
        "publicShareSummary",
    }:
        errors.append("fixtures/frame-api/v1/contract.schema.json: public definitions drifted")

    payloads: dict[str, Any] = {}
    for fixture in files:
        try:
            payload = json.loads(fixture.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"{fixture.relative_to(ROOT)}: invalid JSON ({type(error).__name__})")
            continue
        payloads[fixture.name] = payload
        walk(payload, str(fixture.relative_to(ROOT)), errors)

    for name, payload in payloads.items():
        if name != "error.json" and payload.get("api_version") != {"major": 1}:
            errors.append(f"fixtures/frame-api/v1/{name}: API major drifted")
    unavailable_names = [
        "share.unavailable.json",
        "share.private.json",
        "share.deleted.json",
        "share.failed.json",
    ]
    unavailable_bytes = []
    for name in unavailable_names:
        try:
            unavailable_bytes.append((FIXTURES / name).read_bytes())
        except OSError:
            unavailable_bytes.append(b"")
    if not unavailable_bytes or any(value != unavailable_bytes[0] for value in unavailable_bytes):
        errors.append("fixtures/frame-api/v1: non-public share responses are distinguishable")

    if errors:
        print("public fixture validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(
        f"validated {len(files)} privacy-safe public contract fixtures and one v1 schema"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
