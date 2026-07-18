#!/usr/bin/env python3
"""Verify Frame's token-hidden live release join without exposing the token."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import stat
import sys
import urllib.error
import urllib.request
from typing import Any, Mapping


ORIGINS = {
    "https://frame.engmanager.xyz",
    "https://frame-staging.engmanager.xyz",
}
ROOT = pathlib.Path(__file__).resolve().parents[2]
LATEST_MIGRATION = sorted((ROOT / "apps/control-plane/migrations").glob("*.sql"))[-1].name
MAX_BODY_BYTES = 16 * 1024
GIT_SHA = re.compile(r"^[0-9a-f]{40}$")
SAFE_RELEASE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
MIGRATION = re.compile(r"^[0-9]{4}_[a-z0-9_]+\.sql$")
EXPECTED_FIELDS = {
    "service",
    "status",
    "source_git_sha",
    "contract_major",
    "worker_release",
    "render_deploy",
    "migration_level",
    "portfolio_consumer",
}


class ConformanceError(RuntimeError):
    """The protected release endpoint failed its bounded contract."""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, file_pointer, code, message, headers, new_url):  # type: ignore[no-untyped-def]
        return None


def safe_expected(args: argparse.Namespace) -> dict[str, Any]:
    expected = {
        "source_git_sha": args.expected_source_git_sha,
        "contract_major": 1,
        "worker_release": args.expected_worker_release,
        "render_deploy": args.expected_render_deploy,
        "migration_level": args.expected_migration_level,
        "portfolio_consumer": args.expected_portfolio_consumer,
    }
    if GIT_SHA.fullmatch(str(expected["source_git_sha"])) is None:
        raise ConformanceError("expected source identity must be a full lowercase Git SHA")
    for field in ("worker_release", "render_deploy", "portfolio_consumer"):
        if SAFE_RELEASE.fullmatch(str(expected[field])) is None:
            raise ConformanceError(f"expected {field} is not a bounded safe identifier")
    if MIGRATION.fullmatch(str(expected["migration_level"])) is None:
        raise ConformanceError("expected migration level is invalid")
    return expected


def header(headers: Mapping[str, str], name: str) -> str:
    lowered = name.lower()
    for key, value in headers.items():
        if key.lower() == lowered:
            return value
    return ""


def validate_response(
    status: int,
    headers: Mapping[str, str],
    body: bytes,
    expected: dict[str, Any],
) -> dict[str, Any]:
    if status != 200:
        raise ConformanceError(f"release join returned HTTP {status}")
    if len(body) > MAX_BODY_BYTES:
        raise ConformanceError("release join response exceeded 16 KiB")
    content_type = header(headers, "content-type").split(";", maxsplit=1)[0].strip().lower()
    if content_type != "application/json":
        raise ConformanceError("release join did not return application/json")
    cache_control = header(headers, "cache-control").lower()
    if "no-store" not in {item.strip() for item in cache_control.split(",")}:
        raise ConformanceError("release join is not explicitly no-store")
    if header(headers, "set-cookie"):
        raise ConformanceError("release join set a cookie")
    try:
        value = json.loads(body)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ConformanceError("release join body is not bounded JSON") from error
    if not isinstance(value, dict) or set(value) != EXPECTED_FIELDS:
        raise ConformanceError("release join field inventory is not exact")
    if value.get("service") != "frame-web" or value.get("status") != "joined":
        raise ConformanceError("release join service/status is invalid")
    if value.get("contract_major") != 1:
        raise ConformanceError("release join contract major is incompatible")
    if GIT_SHA.fullmatch(str(value.get("source_git_sha", ""))) is None:
        raise ConformanceError("release join source SHA is invalid")
    for field in ("worker_release", "render_deploy", "portfolio_consumer"):
        if SAFE_RELEASE.fullmatch(str(value.get(field, ""))) is None:
            raise ConformanceError(f"release join {field} is unsafe")
    if MIGRATION.fullmatch(str(value.get("migration_level", ""))) is None:
        raise ConformanceError("release join migration level is unsafe")
    for field, wanted in expected.items():
        if value.get(field) != wanted:
            raise ConformanceError(f"release join does not match expected {field}")
    serialized = json.dumps(value, sort_keys=True).lower()
    if any(
        marker in serialized
        for marker in (
            "authorization",
            "bearer ",
            "cookie",
            "token",
            "x-amz-signature",
            "signed_url",
            "object_key",
            "tenant_id",
            "email",
        )
    ):
        raise ConformanceError("release join contains a forbidden private marker")
    return value


def token_from_file(path: pathlib.Path) -> str:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise ConformanceError(f"cannot inspect diagnostic token file: {error}") from error
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ConformanceError("diagnostic token path must be a regular non-symlink file")
    if metadata.st_mode & 0o077:
        raise ConformanceError("diagnostic token file must not be group/world accessible")
    try:
        payload = path.read_bytes()
    except OSError as error:
        raise ConformanceError(f"cannot read diagnostic token file: {error}") from error
    if len(payload) > 4096:
        raise ConformanceError("diagnostic token file exceeds 4 KiB")
    if payload.endswith(b"\n"):
        payload = payload[:-1]
    if len(payload) < 24 or not payload.isascii() or any(byte <= 0x20 or byte >= 0x7F for byte in payload):
        raise ConformanceError("diagnostic token is not a bounded printable ASCII secret")
    return payload.decode("ascii")


def fetch(origin: str, token: str) -> tuple[int, Mapping[str, str], bytes]:
    if origin not in ORIGINS:
        raise ConformanceError("origin is not in the fixed release-join allowlist")
    request = urllib.request.Request(
        f"{origin}/health/release",
        headers={
            "Accept": "application/json",
            "Authorization": f"Bearer {token}",
            "User-Agent": "frame-release-join-conformance/1",
        },
        method="GET",
    )
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    try:
        with opener.open(request, timeout=10) as response:
            body = response.read(MAX_BODY_BYTES + 1)
            return response.status, dict(response.headers.items()), body
    except urllib.error.HTTPError as error:
        raise ConformanceError(f"release join returned HTTP {error.code}") from error
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        raise ConformanceError(f"release join request failed: {error.__class__.__name__}") from error


def self_test() -> list[str]:
    expected = {
        "source_git_sha": "1" * 40,
        "contract_major": 1,
        "worker_release": "worker-1111111",
        "render_deploy": "render-deploy-1",
        "migration_level": LATEST_MIGRATION,
        "portfolio_consumer": "portfolio-aaaaaaa",
    }
    value = {"service": "frame-web", "status": "joined", **expected}
    headers = {"content-type": "application/json; charset=utf-8", "cache-control": "no-store"}
    body = json.dumps(value, separators=(",", ":")).encode()
    validate_response(200, headers, body, expected)

    cases: list[tuple[str, int, dict[str, str], dict[str, Any], dict[str, Any]]] = []
    missing = dict(value)
    missing.pop("render_deploy")
    cases.append(("missing_field", 200, dict(headers), missing, expected))
    incompatible = dict(value)
    incompatible["contract_major"] = 2
    cases.append(("contract_drift", 200, dict(headers), incompatible, expected))
    mismatch = dict(value)
    mismatch["worker_release"] = "worker-other"
    cases.append(("release_mismatch", 200, dict(headers), mismatch, expected))
    unsafe = dict(value)
    unsafe["portfolio_consumer"] = "portfolio?query"
    cases.append(("unsafe_identifier", 200, dict(headers), unsafe, expected))
    cacheable = dict(headers)
    cacheable["cache-control"] = "public, max-age=60"
    cases.append(("cacheable", 200, cacheable, dict(value), expected))
    cookie = dict(headers)
    cookie["set-cookie"] = "frame_session=forbidden"
    cases.append(("cookie", 200, cookie, dict(value), expected))
    cases.append(("wrong_status", 503, dict(headers), dict(value), expected))

    rejected: list[str] = []
    for name, status, case_headers, case_value, case_expected in cases:
        try:
            validate_response(
                status,
                case_headers,
                json.dumps(case_value, separators=(",", ":")).encode(),
                case_expected,
            )
        except ConformanceError:
            rejected.append(name)
        else:
            raise ConformanceError(f"release join self-test did not reject {name}")
    return rejected


def safe_write(path: pathlib.Path, value: dict[str, Any]) -> None:
    if path.is_symlink():
        raise ConformanceError("evidence output may not be a symbolic link")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", choices=sorted(ORIGINS))
    parser.add_argument("--token-file", type=pathlib.Path)
    parser.add_argument("--expected-source-git-sha")
    parser.add_argument("--expected-worker-release")
    parser.add_argument("--expected-render-deploy")
    parser.add_argument("--expected-migration-level")
    parser.add_argument("--expected-portfolio-consumer")
    parser.add_argument("--evidence", type=pathlib.Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    live_values = (
        args.origin,
        args.token_file,
        args.expected_source_git_sha,
        args.expected_worker_release,
        args.expected_render_deploy,
        args.expected_migration_level,
        args.expected_portfolio_consumer,
        args.evidence,
    )
    if args.self_test:
        if any(value is not None for value in live_values):
            parser.error("--self-test cannot be combined with live arguments")
    elif any(value is None for value in live_values):
        parser.error("live verification requires origin, token file, every expected field, and evidence")
    return args


def main() -> int:
    args = arguments()
    try:
        if args.self_test:
            rejected = self_test()
            print(f"release join conformance self-test passed: rejected {len(rejected)} unsafe cases")
            return 0
        expected = safe_expected(args)
        token = token_from_file(args.token_file)
        status, headers, body = fetch(args.origin, token)
        value = validate_response(status, headers, body, expected)
        report = {
            "schema_version": 1,
            "evidence_kind": "frame_release_join_live_v1",
            "origin": args.origin,
            "source_git_sha": value["source_git_sha"],
            "contract_major": value["contract_major"],
            "worker_release": value["worker_release"],
            "render_deploy": value["render_deploy"],
            "migration_level": value["migration_level"],
            "portfolio_consumer": value["portfolio_consumer"],
            "body_sha256": hashlib.sha256(body).hexdigest(),
            "no_store": True,
            "set_cookie": False,
            "live_endpoint_observed": True,
            "production_authority_changed": False,
        }
        safe_write(args.evidence, report)
        print("release join conformance passed; bounded safe evidence written")
        return 0
    except (ConformanceError, KeyError, TypeError, ValueError) as error:
        print(f"release join conformance failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
