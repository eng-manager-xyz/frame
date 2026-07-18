#!/usr/bin/env python3
"""Run Frame's deterministic, provider-free semantic walking slice.

The journey uses only Python's standard library, loopback HTTP, a temporary
SQLite database, and checked-in synthetic bytes. It deliberately models the
provider seams; it does not emulate or claim compatibility with Cloudflare,
Render, a browser, GStreamer, or production infrastructure.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parents[2]
FIXTURE_DIR = ROOT / "fixtures" / "hermetic" / "v1"
FIXTURE_MANIFEST = FIXTURE_DIR / "manifest.json"
MEDIA_FIXTURE = FIXTURE_DIR / "synthetic.webm"
FAKE_SERVER = ROOT / "scripts" / "ci" / "hermetic-fake.py"
WEB_ORIGIN = "https://web.frame.hermetic.invalid"
ACTOR_HEADERS = {"X-Hermetic-Actor": "synthetic-owner"}
PROVIDER_VARIABLES = (
    "CLOUDFLARE_API_TOKEN",
    "CLOUDFLARE_ACCOUNT_ID",
    "CLOUDFLARE_D1_DATABASE_ID",
    "CLOUDFLARE_R2_ACCESS_KEY_ID",
    "CLOUDFLARE_R2_SECRET_ACCESS_KEY",
    "DATABASE_URL",
    "MYSQL_URL",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "RENDER_API_KEY",
)
REDACTIONS = (
    re.compile(
        r"(?i)(authorization|cookie|x-amz-signature|api[_-]?token)\s*[:=]\s*\S+"
    ),
    re.compile(r"(?i)bearer\s+\S+"),
    re.compile(r"tenants/[A-Za-z0-9_./-]+"),
    re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+"),
)
UUID_RE = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
)


class JourneyFailure(RuntimeError):
    def __init__(self, check: str, message: str) -> None:
        super().__init__(message)
        self.check = check


@dataclass(frozen=True)
class HttpResult:
    status: int
    headers: dict[str, str]
    body: bytes

    def json(self) -> dict[str, Any]:
        try:
            value = json.loads(self.body)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise JourneyFailure("json-contract", "response was not valid JSON") from error
        if not isinstance(value, dict):
            raise JourneyFailure("json-contract", "response JSON was not an object")
        return value


class Deadline:
    def __init__(self, seconds: float) -> None:
        self.end = time.monotonic() + seconds

    def remaining(self, ceiling: float | None = None) -> float:
        value = self.end - time.monotonic()
        if value <= 0:
            raise JourneyFailure("global-timeout", "hermetic journey exceeded its deadline")
        return value if ceiling is None else min(value, ceiling)


class ManagedProcess:
    def __init__(
        self,
        name: str,
        args: list[str],
        env: dict[str, str],
        log_path: Path,
    ) -> None:
        self.name = name
        self.log_path = log_path
        self.log_handle = log_path.open("wb")
        self.process = subprocess.Popen(
            args,
            cwd=ROOT,
            env=env,
            stdin=subprocess.DEVNULL,
            stdout=self.log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )

    def ensure_running(self) -> None:
        code = self.process.poll()
        if code is not None:
            raise JourneyFailure(
                f"{self.name}-startup",
                f"{self.name} exited with {code}: {sanitized_tail(self.log_path)}",
            )

    def stop(self) -> None:
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(self.process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self.process.wait(timeout=2)
        self.log_handle.close()


def require(condition: bool, check: str, message: str) -> None:
    if not condition:
        raise JourneyFailure(check, message)


def ok(label: str) -> None:
    print(f"[ok] {label}", flush=True)


def sanitize(value: str) -> str:
    value = value.replace(str(ROOT), "<workspace>")
    home = str(Path.home())
    if home:
        value = value.replace(home, "<home>")
    for pattern in REDACTIONS:
        value = pattern.sub("<redacted>", value)
    return value[-4000:]


def sanitized_tail(path: Path) -> str:
    try:
        data = path.read_bytes()[-16_384:].decode("utf-8", "replace")
    except OSError:
        return "<log unavailable>"
    return sanitize(data)


def refuse_provider_state() -> None:
    present = [name for name in PROVIDER_VARIABLES if os.environ.get(name)]
    if os.environ.get("FRAME_DEPLOYMENT") == "production":
        present.append("FRAME_DEPLOYMENT=production")
    require(
        not present,
        "provider-isolation",
        "provider or production configuration is present; refusing hermetic run",
    )


def load_and_validate_fixture() -> dict[str, Any]:
    try:
        fixture = json.loads(FIXTURE_MANIFEST.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise JourneyFailure("fixture-manifest", "fixture manifest is invalid") from error
    require(isinstance(fixture, dict), "fixture-manifest", "manifest must be an object")
    required = {
        "schema_version",
        "tenant_id",
        "video_id",
        "upload_id",
        "job_id",
        "object_key",
        "content_type",
        "bytes",
        "sha256",
        "provenance",
    }
    require(set(fixture) == required, "fixture-manifest", "manifest fields drifted")
    require(fixture["schema_version"] == 1, "fixture-manifest", "schema drifted")
    for field in ("tenant_id", "video_id", "upload_id", "job_id"):
        require(
            isinstance(fixture[field], str) and UUID_RE.fullmatch(fixture[field]) is not None,
            "fixture-identifiers",
            f"fixture {field} is not a canonical UUID",
        )
    provenance = fixture["provenance"]
    require(
        isinstance(provenance, dict)
        and provenance
        == {
            "kind": "generated-synthetic",
            "contains_personal_data": False,
            "contains_production_data": False,
            "decodable_media": False,
        },
        "fixture-provenance",
        "fixture provenance must identify generated, non-production, non-media data",
    )
    data = MEDIA_FIXTURE.read_bytes()
    require(
        isinstance(fixture["bytes"], int)
        and fixture["bytes"] > 0
        and len(data) == fixture["bytes"]
        and isinstance(fixture["sha256"], str)
        and re.fullmatch(r"[0-9a-f]{64}", fixture["sha256"]) is not None
        and hashlib.sha256(data).hexdigest() == fixture["sha256"],
        "fixture-integrity",
        "synthetic fixture length or digest drifted",
    )
    prefix = (
        f"tenants/{fixture['tenant_id']}/videos/{fixture['video_id']}/derivatives/"
    )
    key = fixture["object_key"]
    require(
        isinstance(key, str)
        and key.startswith(prefix)
        and not any(value in key for value in ("..", "\\", "?", "#", "%")),
        "fixture-object-key",
        "synthetic object key violates the tenant-safe shape",
    )
    require(
        fixture["content_type"] == "video/webm" and FAKE_SERVER.is_file(),
        "fixture-files",
        "hermetic content type or semantic server drifted",
    )
    return fixture


def child_env() -> dict[str, str]:
    env = os.environ.copy()
    for name in PROVIDER_VARIABLES:
        env.pop(name, None)
    env["FRAME_DEPLOYMENT"] = "hermetic"
    env["PYTHONHASHSEED"] = "0"
    return env


def http_request(
    method: str,
    url: str,
    deadline: Deadline,
    *,
    headers: dict[str, str] | None = None,
    json_body: dict[str, Any] | None = None,
    body: bytes | None = None,
) -> HttpResult:
    request_headers = dict(headers or {})
    if json_body is not None:
        require(body is None, "http-request", "request cannot have two bodies")
        body = json.dumps(json_body, sort_keys=True, separators=(",", ":")).encode()
        request_headers["Content-Type"] = "application/json"
    request = Request(url, data=body, headers=request_headers, method=method)
    try:
        response = urlopen(request, timeout=deadline.remaining(3))
        with response:
            return HttpResult(
                response.status,
                {key.lower(): value for key, value in response.headers.items()},
                response.read(),
            )
    except HTTPError as error:
        return HttpResult(
            error.code,
            {key.lower(): value for key, value in error.headers.items()},
            error.read(),
        )
    except (URLError, TimeoutError, ConnectionError, OSError) as error:
        raise JourneyFailure("http-transport", "loopback HTTP request failed") from error


def assert_status(result: HttpResult, status: int, check: str) -> None:
    require(result.status == status, check, f"expected HTTP {status}, got {result.status}")


def assert_no_store(result: HttpResult, check: str) -> None:
    value = result.headers.get("cache-control", "").lower()
    require("no-store" in value, check, "dynamic response was cacheable")


def assert_semantic_fake(result: HttpResult, check: str) -> None:
    require(
        result.headers.get("x-hermetic-component") == "provider-semantic-fake",
        check,
        "semantic response was not explicitly labeled as simulated",
    )


def assert_redacted(value: Any, fixture: dict[str, Any], check: str) -> None:
    serialized = json.dumps(value, sort_keys=True)
    forbidden = (
        fixture["object_key"],
        fixture["sha256"],
        "owner@frame.invalid",
        "source_path",
        "playback_path",
        str(ROOT),
    )
    require(
        not any(marker in serialized for marker in forbidden),
        check,
        "public or error response exposed a storage, identity, or filesystem detail",
    )


def wait_ready(
    process: ManagedProcess, origin: str, deadline: Deadline
) -> None:
    end = time.monotonic() + deadline.remaining(5)
    while time.monotonic() < end:
        process.ensure_running()
        try:
            result = http_request("GET", f"{origin}/__hermetic/health", deadline)
            if result.status == 200:
                body = result.json()
                if body == {
                    "status": "ready",
                    "component": "provider-semantic-fake",
                    "simulated": True,
                }:
                    assert_semantic_fake(result, "semantic-fake-readiness")
                    assert_no_store(result, "semantic-fake-readiness-cache")
                    return
        except JourneyFailure as error:
            if error.check != "http-transport":
                raise
        time.sleep(0.05)
    raise JourneyFailure("semantic-fake-readiness", "semantic fake did not become ready")


def fake_process(
    temp: Path,
    inject_private_cache_hit: bool,
    deadline: Deadline,
) -> tuple[ManagedProcess, str, Path]:
    suffix = "defect" if inject_private_cache_hit else "control"
    state = temp / f"semantic-fake-{suffix}"
    ready_file = temp / f"semantic-fake-{suffix}.port"
    args = [
        sys.executable,
        "-I",
        str(FAKE_SERVER),
        "--port",
        "0",
        "--ready-file",
        str(ready_file),
        "--state-dir",
        str(state),
        "--fixture",
        str(FIXTURE_MANIFEST),
        "--web-origin",
        WEB_ORIGIN,
    ]
    if inject_private_cache_hit:
        args.append("--inject-private-cache-hit")
    process = ManagedProcess(
        f"semantic-fake-{suffix}",
        args,
        child_env(),
        temp / f"semantic-fake-{suffix}.log",
    )
    end = time.monotonic() + deadline.remaining(3)
    while time.monotonic() < end and not ready_file.is_file():
        process.ensure_running()
        time.sleep(0.02)
    require(ready_file.is_file(), "semantic-fake-port", "semantic fake did not bind")
    try:
        port = int(ready_file.read_text(encoding="ascii").strip())
    except (OSError, ValueError) as error:
        process.stop()
        raise JourneyFailure("semantic-fake-port", "semantic fake port is invalid") from error
    require(1024 <= port <= 65535, "semantic-fake-port", "semantic fake port is unsafe")
    return process, f"http://127.0.0.1:{port}", state


def run_semantic_journey(
    origin: str,
    state_dir: Path,
    fixture: dict[str, Any],
    deadline: Deadline,
) -> None:
    intent_payload = {
        "schema_version": 1,
        "tenant_id": fixture["tenant_id"],
        "video_id": fixture["video_id"],
        "expected_bytes": fixture["bytes"],
        "content_type": fixture["content_type"],
        "checksum_sha256": fixture["sha256"],
    }
    denied = http_request(
        "POST", f"{origin}/api/v1/uploads/intents", deadline, json_body=intent_payload
    )
    assert_status(denied, 401, "auth-boundary")
    assert_no_store(denied, "auth-boundary-cache")
    assert_semantic_fake(denied, "auth-boundary-label")
    assert_redacted(denied.json(), fixture, "auth-boundary-redaction")

    cross_tenant_payload = dict(intent_payload)
    cross_tenant_payload["tenant_id"] = "018f47a6-7b1c-7f55-8f39-8f8a86900999"
    cross_tenant = http_request(
        "POST",
        f"{origin}/api/v1/uploads/intents",
        deadline,
        headers=ACTOR_HEADERS,
        json_body=cross_tenant_payload,
    )
    assert_status(cross_tenant, 422, "tenant-boundary")
    assert_redacted(cross_tenant.json(), fixture, "tenant-boundary-redaction")

    intent = http_request(
        "POST",
        f"{origin}/api/v1/uploads/intents",
        deadline,
        headers=ACTOR_HEADERS,
        json_body=intent_payload,
    )
    assert_status(intent, 201, "upload-intent")
    assert_semantic_fake(intent, "upload-intent-label")
    intent_body = intent.json()
    upload_url = intent_body.get("upload_url")
    require(
        intent_body.get("simulated") is True
        and intent_body.get("provider") == "hermetic-r2-semantic-fake"
        and isinstance(upload_url, str)
        and upload_url == f"{origin}/fake-r2/uploads/{fixture['upload_id']}"
        and not upload_url.startswith(WEB_ORIGIN),
        "direct-upload-boundary",
        "upload intent did not target the labeled isolated object seam",
    )
    assert_no_store(intent, "upload-intent-cache")

    replay = http_request(
        "POST",
        f"{origin}/api/v1/uploads/intents",
        deadline,
        headers=ACTOR_HEADERS,
        json_body=intent_payload,
    )
    assert_status(replay, 201, "upload-intent-replay")
    require(replay.json() == intent_body, "upload-intent-replay", "intent replay drifted")

    preflight = http_request(
        "OPTIONS", upload_url, deadline, headers={"Origin": WEB_ORIGIN}
    )
    assert_status(preflight, 204, "upload-cors")
    require(
        preflight.headers.get("access-control-allow-origin") == WEB_ORIGIN
        and "PUT" in preflight.headers.get("access-control-allow-methods", "")
        and "x-frame-checksum"
        in preflight.headers.get("access-control-allow-headers", "").lower(),
        "upload-cors",
        "modeled object CORS rejected the configured web origin",
    )
    hostile_preflight = http_request(
        "OPTIONS",
        upload_url,
        deadline,
        headers={"Origin": "https://hostile.invalid"},
    )
    assert_status(hostile_preflight, 204, "hostile-cors")
    require(
        "access-control-allow-origin" not in hostile_preflight.headers,
        "hostile-cors",
        "modeled object CORS reflected an untrusted origin",
    )

    fixture_bytes = MEDIA_FIXTURE.read_bytes()
    hostile_put = http_request(
        "PUT",
        upload_url,
        deadline,
        headers={
            "Origin": "https://hostile.invalid",
            "Content-Type": fixture["content_type"],
            "X-Frame-Checksum": fixture["sha256"],
        },
        body=fixture_bytes,
    )
    assert_status(hostile_put, 403, "hostile-upload")
    mismatch = http_request(
        "PUT",
        upload_url,
        deadline,
        headers={
            "Origin": WEB_ORIGIN,
            "Content-Type": fixture["content_type"],
            "X-Frame-Checksum": "0" * 64,
        },
        body=fixture_bytes,
    )
    assert_status(mismatch, 400, "upload-checksum-header")
    assert_redacted(mismatch.json(), fixture, "upload-checksum-redaction")

    put = http_request(
        "PUT",
        upload_url,
        deadline,
        headers={
            "Origin": WEB_ORIGIN,
            "Content-Type": fixture["content_type"],
            "X-Frame-Checksum": fixture["sha256"],
        },
        body=fixture_bytes,
    )
    assert_status(put, 201, "object-put")
    etag = put.headers.get("etag")
    require(bool(etag), "object-put", "modeled object PUT omitted ETag")

    share_url = f"{origin}/api/v1/public/shares/{fixture['video_id']}"
    before_finalize = http_request("GET", share_url, deadline)
    assert_status(before_finalize, 200, "share-before-finalize")
    require(
        before_finalize.json().get("availability") == "unavailable",
        "share-before-finalize",
        "unfinalized upload became public",
    )

    wrong_finalize = http_request(
        "POST",
        f"{origin}/api/v1/uploads/{fixture['upload_id']}/finalize",
        deadline,
        headers=ACTOR_HEADERS,
        json_body={"etag": '"wrong"'},
    )
    assert_status(wrong_finalize, 422, "finalize-proof")
    finalize = http_request(
        "POST",
        f"{origin}/api/v1/uploads/{fixture['upload_id']}/finalize",
        deadline,
        headers=ACTOR_HEADERS,
        json_body={"etag": etag},
    )
    assert_status(finalize, 202, "finalize")
    require(
        finalize.json().get("state") == "processing"
        and finalize.json().get("simulated") is True,
        "finalize",
        "finalize did not enter the labeled processing state",
    )
    processing = http_request("GET", share_url, deadline)
    assert_status(processing, 200, "processing-share")
    processing_body = processing.json()
    require(
        processing_body.get("availability") == "processing"
        and processing_body.get("title") is None
        and processing_body.get("playback") is None,
        "processing-share",
        "processing share exposed premature metadata or playback",
    )
    assert_redacted(processing_body, fixture, "processing-share-redaction")

    managed_failure = http_request(
        "POST",
        f"{origin}/api/v1/media-jobs/{fixture['job_id']}/process",
        deadline,
        headers=ACTOR_HEADERS,
        json_body={"executor": "media_fake"},
    )
    assert_status(managed_failure, 503, "managed-media-failure")
    failure_body = managed_failure.json()
    require(
        failure_body.get("code") == "media_fake_unavailable"
        and failure_body.get("retry") == "later",
        "managed-media-failure",
        "injected managed-media failure did not use the stable retry contract",
    )
    assert_no_store(managed_failure, "managed-media-failure-cache")
    assert_redacted(failure_body, fixture, "managed-media-failure-redaction")

    fallback = http_request(
        "POST",
        f"{origin}/api/v1/media-jobs/{fixture['job_id']}/process",
        deadline,
        headers=ACTOR_HEADERS,
        json_body={"executor": "native_fallback"},
    )
    assert_status(fallback, 200, "native-fallback")
    require(
        fallback.json().get("executor") == "native_fallback"
        and fallback.json().get("state") == "succeeded"
        and fallback.json().get("simulated") is True,
        "native-fallback",
        "modeled fallback did not publish the derivative",
    )

    public_share = http_request("GET", share_url, deadline)
    assert_status(public_share, 200, "public-share")
    public_body = public_share.json()
    require(
        public_body.get("availability") == "public"
        and public_body.get("playback", {}).get("supports_range") is True
        and public_body.get("canonical_url")
        == f"{WEB_ORIGIN}/s/{fixture['video_id']}",
        "public-share",
        "modeled public share was not playable",
    )
    assert_no_store(public_share, "public-share-cache")
    assert_redacted(public_body, fixture, "public-share-redaction")

    media_url = f"{share_url}/media"
    ranged = http_request("GET", media_url, deadline, headers={"Range": "bytes=11-35"})
    assert_status(ranged, 206, "range")
    require(
        ranged.body == fixture_bytes[11:36]
        and ranged.headers.get("content-range") == f"bytes 11-35/{len(fixture_bytes)}"
        and ranged.headers.get("accept-ranges") == "bytes",
        "range",
        "modeled playback Range response drifted",
    )
    assert_no_store(ranged, "range-cache")
    suffix = http_request("GET", media_url, deadline, headers={"Range": "bytes=-9"})
    assert_status(suffix, 206, "suffix-range")
    require(suffix.body == fixture_bytes[-9:], "suffix-range", "suffix Range drifted")
    bad_range = http_request("GET", media_url, deadline, headers={"Range": "bytes=5000-"})
    assert_status(bad_range, 416, "invalid-range")
    require(
        bad_range.headers.get("content-range") == f"bytes */{len(fixture_bytes)}",
        "invalid-range",
        "invalid Range omitted the total length",
    )
    assert_no_store(bad_range, "invalid-range-cache")
    head = http_request("HEAD", media_url, deadline)
    assert_status(head, 200, "media-head")
    require(
        not head.body and head.headers.get("content-length") == str(len(fixture_bytes)),
        "media-head",
        "modeled media HEAD drifted",
    )

    asset_url = f"{origin}/assets/app.0123456789abcdef.js"
    first_asset = http_request("GET", asset_url, deadline)
    second_asset = http_request("GET", asset_url, deadline)
    require(
        first_asset.headers.get("x-hermetic-cache") == "MISS"
        and second_asset.headers.get("x-hermetic-cache") == "HIT"
        and "immutable" in second_asset.headers.get("cache-control", ""),
        "immutable-asset-cache",
        "fingerprinted synthetic asset did not transition MISS to HIT",
    )

    full_media = http_request("GET", media_url, deadline)
    assert_status(full_media, 200, "public-media")
    require(
        full_media.body == fixture_bytes
        and full_media.headers.get("x-hermetic-cache") == "BYPASS",
        "public-media",
        "dynamic media did not bypass the modeled edge cache",
    )
    assert_no_store(full_media, "public-media-cache")
    privacy = http_request(
        "PATCH",
        f"{origin}/api/v1/videos/{fixture['video_id']}/privacy",
        deadline,
        headers=ACTOR_HEADERS,
        json_body={"privacy": "private"},
    )
    assert_status(privacy, 200, "privacy-transition")
    private_share = http_request("GET", share_url, deadline)
    private_media = http_request("GET", media_url, deadline)
    private_body = private_share.json()
    require(
        private_share.status == 200
        and private_body.get("availability") == "unavailable"
        and private_body.get("title") is None
        and private_media.status == 404
        and private_media.headers.get("x-hermetic-cache") != "HIT"
        and privacy.json().get("cache_purged") is True,
        "privacy-cache-leak",
        "private media remained public or cacheable after the privacy transition",
    )
    assert_no_store(private_share, "private-share-cache")
    assert_no_store(private_media, "private-media-cache")
    assert_redacted(private_body, fixture, "private-share-redaction")
    verify_database(state_dir / "semantic-d1.sqlite3", fixture)


def verify_database(path: Path, fixture: dict[str, Any]) -> None:
    require(path.is_file(), "state-database", "temporary semantic database is missing")
    with sqlite3.connect(path) as db:
        integrity = db.execute("PRAGMA integrity_check").fetchone()
        upload = db.execute(
            "SELECT state, expected_bytes, checksum_sha256, source_path FROM uploads WHERE id = ?",
            (fixture["upload_id"],),
        ).fetchone()
        video = db.execute(
            "SELECT privacy, state, bytes, checksum_sha256, playback_path FROM videos WHERE id = ?",
            (fixture["video_id"],),
        ).fetchone()
        job = db.execute(
            "SELECT state, attempt, selected_executor, error_code FROM jobs WHERE id = ?",
            (fixture["job_id"],),
        ).fetchone()
        objects = db.execute(
            "SELECT role, state, bytes, checksum_sha256, path FROM objects "
            "WHERE video_id = ? ORDER BY role",
            (fixture["video_id"],),
        ).fetchall()
    require(integrity == ("ok",), "state-integrity", "SQLite integrity check failed")
    require(
        upload is not None and upload[:3] == ("finalized", fixture["bytes"], fixture["sha256"]),
        "state-upload",
        "upload row did not reconcile",
    )
    require(
        video is not None
        and video[:4] == ("private", "ready", fixture["bytes"], fixture["sha256"]),
        "state-video",
        "video row did not reconcile",
    )
    require(
        job == ("succeeded", 2, "native_fallback", None),
        "state-job",
        "job row did not record failure and fallback completion",
    )
    require(
        len(objects) == 2
        and {row[0] for row in objects} == {"source", "playback"}
        and all(
            row[1:4] == ("available", fixture["bytes"], fixture["sha256"])
            for row in objects
        ),
        "state-objects",
        "source and playback rows did not reconcile",
    )
    for row in objects:
        artifact = Path(row[4])
        try:
            data = artifact.read_bytes()
        except OSError as error:
            raise JourneyFailure("object-integrity", "semantic object is missing") from error
        require(
            len(data) == fixture["bytes"]
            and hashlib.sha256(data).hexdigest() == fixture["sha256"],
            "object-integrity",
            "source or derivative bytes drifted",
        )


def run(args: argparse.Namespace) -> None:
    refuse_provider_state()
    fixture = load_and_validate_fixture()
    ok("synthetic fixture provenance, scope, and checksum")
    deadline = Deadline(args.timeout)
    with tempfile.TemporaryDirectory(prefix="frame-hermetic-") as temporary:
        temp = Path(temporary)
        control, control_origin, control_state = fake_process(temp, False, deadline)
        try:
            wait_ready(control, control_origin, deadline)
            run_semantic_journey(control_origin, control_state, fixture, deadline)
        finally:
            control.stop()
        ok(
            "simulated upload, finalize, managed failure, fallback, share, Range, cache, "
            "privacy, and state reconciliation"
        )

        defect, defect_origin, defect_state = fake_process(temp, True, deadline)
        try:
            wait_ready(defect, defect_origin, deadline)
            try:
                run_semantic_journey(defect_origin, defect_state, fixture, deadline)
            except JourneyFailure as error:
                require(
                    error.check == "privacy-cache-leak",
                    "failure-self-test",
                    f"seeded defect triggered the wrong check: {error.check}",
                )
            else:
                raise JourneyFailure(
                    "failure-self-test", "seeded private-cache defect was not detected"
                )
        finally:
            defect.stop()
        ok("deliberate private-cache defect was detected by the same journey")
    print(
        "PASS hermetic semantic journey; runtime/provider compatibility claims: none",
        flush=True,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--timeout",
        type=float,
        default=30.0,
        help="total hard deadline in seconds (default: 30)",
    )
    args = parser.parse_args()
    if not 5 <= args.timeout <= 120:
        parser.error("--timeout must be between 5 and 120 seconds")
    try:
        run(args)
    except JourneyFailure as error:
        print(f"FAIL {error.check}: {sanitize(str(error))}", file=sys.stderr, flush=True)
        return 1
    except KeyboardInterrupt:
        print("FAIL interrupted: hermetic journey interrupted", file=sys.stderr, flush=True)
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
