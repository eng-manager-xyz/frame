#!/usr/bin/env python3
"""Credential-free, loopback-only portfolio-to-Frame contract journey.

This is a semantic fake, not a browser or provider emulator. It deliberately
starts two processes on distinct loopback origins and validates the public
fixture, failure, cache, cookie, navigation, media, and degradation contracts
that can be proved without another repository or privileged infrastructure.
"""

from __future__ import annotations

import argparse
import hashlib
import html
import http.cookiejar
import json
import os
import re
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from html.parser import HTMLParser
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Callable
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import (
    HTTPCookieProcessor,
    ProxyHandler,
    Request,
    build_opener,
)


ROOT = Path(__file__).resolve().parents[2]
CONTRACT_PATH = ROOT / "fixtures" / "cross-repo-preview" / "v1" / "contract.json"
CAPTION_PATH = ROOT / "fixtures" / "cross-repo-preview" / "v1" / "captions.en.vtt"
MAX_RESPONSE_BYTES = 64 * 1024
PORTFOLIO_HANDLER_BUDGET_SECONDS = 0.180
PORTFOLIO_POLL_TIMEOUT_SECONDS = 0.120
FRAME_SLOW_SECONDS = 0.400
PROVIDER_VARIABLES = (
    "CLOUDFLARE_API_TOKEN",
    "CLOUDFLARE_ACCOUNT_ID",
    "CLOUDFLARE_D1_DATABASE_ID",
    "CLOUDFLARE_R2_ACCESS_KEY_ID",
    "CLOUDFLARE_R2_SECRET_ACCESS_KEY",
    "RENDER_API_KEY",
    "DATABASE_URL",
    "MYSQL_URL",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
)
DEFECT_EXPECTATIONS = {
    "shared-cookie-domain": "frame-host-only-cookie",
    "private-cache-hit": "private-cache-isolation",
    "range-off-by-one": "range-contract",
    "unavailable-title-leak": "unavailable-privacy",
    "handler-path-upstream-fetch": "portfolio-latency",
    "audit-sensitive-field": "audit-redaction",
}
SAFE_RELEASE_RE = re.compile(r"^[A-Za-z0-9._-]{1,64}$")
RANGE_RE = re.compile(r"^bytes=(\d*)-(\d*)$")


class HarnessFailure(RuntimeError):
    def __init__(self, check: str, message: str) -> None:
        super().__init__(message)
        self.check = check


def require(condition: bool, check: str, message: str) -> None:
    if not condition:
        raise HarnessFailure(check, message)


def emit(enabled: bool, label: str) -> None:
    if enabled:
        print(f"[ok] {label}", flush=True)


def read_json_object(path: Path, check: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise HarnessFailure(check, f"{path.name} is not valid UTF-8 JSON") from error
    require(isinstance(value, dict), check, f"{path.name} must contain an object")
    return value


@dataclass(frozen=True)
class FixtureSet:
    contract: dict[str, Any]
    json_bytes: dict[str, bytes]
    media: bytes
    captions: bytes


def load_fixtures() -> FixtureSet:
    contract = read_json_object(CONTRACT_PATH, "preview-fixture")
    require(
        set(contract)
        == {
            "schema_version",
            "evidence_class",
            "provider_compatibility",
            "public_fixture_directory",
            "synthetic_media",
            "api_no_store_headers",
            "portfolio_public_cache_control",
            "fingerprinted_asset_cache_control",
            "negative_controls",
        },
        "preview-fixture",
        "preview contract fields drifted",
    )
    require(
        contract["schema_version"] == 1
        and contract["evidence_class"] == "local_semantic_fake"
        and contract["provider_compatibility"] is False,
        "preview-fixture",
        "preview evidence classification drifted",
    )
    require(
        contract["public_fixture_directory"] == "fixtures/frame-api/v1",
        "preview-fixture",
        "public fixture authority drifted",
    )
    require(
        contract["negative_controls"] == list(DEFECT_EXPECTATIONS),
        "preview-fixture",
        "negative-control inventory drifted",
    )

    media_meta = contract["synthetic_media"]
    require(
        isinstance(media_meta, dict)
        and set(media_meta) == {"path", "bytes", "sha256"}
        and media_meta["path"] == "fixtures/hermetic/v1/synthetic.webm",
        "preview-fixture",
        "synthetic media metadata drifted",
    )
    media_path = ROOT / media_meta["path"]
    media = media_path.read_bytes()
    require(
        media_meta["bytes"] == len(media) == 127
        and isinstance(media_meta["sha256"], str)
        and hashlib.sha256(media).hexdigest() == media_meta["sha256"],
        "preview-fixture",
        "synthetic media integrity check failed",
    )
    captions = CAPTION_PATH.read_bytes()
    require(
        captions.startswith(b"WEBVTT\n\n") and len(captions) <= 4_096,
        "preview-fixture",
        "caption fixture is invalid or unexpectedly large",
    )

    fixture_dir = ROOT / contract["public_fixture_directory"]
    names = (
        "health.ok.json",
        "share.public.json",
        "share.processing.json",
        "share.unavailable.json",
        "share.private.json",
        "share.deleted.json",
        "share.failed.json",
        "error.json",
    )
    json_bytes = {name: (fixture_dir / name).read_bytes() for name in names}
    for name, body in json_bytes.items():
        require(
            len(body) <= MAX_RESPONSE_BYTES
            and isinstance(json.loads(body), dict),
            "preview-fixture",
            f"{name} is not a bounded JSON object",
        )
    unavailable = json_bytes["share.unavailable.json"]
    require(
        all(
            json_bytes[name] == unavailable
            for name in ("share.private.json", "share.deleted.json", "share.failed.json")
        ),
        "preview-fixture",
        "non-public fixtures are not byte-identical",
    )

    headers = contract["api_no_store_headers"]
    require(
        isinstance(headers, dict)
        and all(
            isinstance(key, str)
            and key == key.lower()
            and isinstance(value, str)
            and value
            for key, value in headers.items()
        )
        and headers.get("cache-control") == "no-store, max-age=0"
        and headers.get("vary") == "Origin",
        "preview-fixture",
        "API cache/security header contract drifted",
    )
    return FixtureSet(contract, json_bytes, media, captions)


def refuse_provider_state() -> None:
    present = [name for name in PROVIDER_VARIABLES if os.environ.get(name)]
    if os.environ.get("FRAME_DEPLOYMENT") == "production":
        present.append("FRAME_DEPLOYMENT")
    require(
        not present,
        "credential-isolation",
        "provider or production configuration is present; refusing local preview",
    )


def child_environment() -> dict[str, str]:
    environment = os.environ.copy()
    for name in PROVIDER_VARIABLES:
        environment.pop(name, None)
    environment["FRAME_DEPLOYMENT"] = "cross-repo-local-semantic-fake"
    environment["PYTHONHASHSEED"] = "0"
    environment["PYTHONDONTWRITEBYTECODE"] = "1"
    return environment


def atomic_write(path: Path, value: str) -> None:
    temporary = path.with_name(f".{path.name}.tmp")
    temporary.write_text(value, encoding="utf-8")
    os.replace(temporary, path)


def cookie_names(raw: str | None) -> list[str]:
    if not raw:
        return []
    names: set[str] = set()
    for part in raw.split(";"):
        candidate = part.strip().split("=", maxsplit=1)[0]
        if candidate and re.fullmatch(r"[A-Za-z0-9_\-]{1,64}", candidate):
            names.add(candidate)
    return sorted(names)


class PreviewHTTPServer(ThreadingHTTPServer):
    daemon_threads = True
    # A semantic deploy deliberately restarts Frame on the exact paired origin.
    # The orchestrator proves the old listener is closed before this socket is
    # created, so address reuse covers TCP TIME_WAIT without overlapping owners.
    allow_reuse_address = True

    def __init__(
        self,
        address: tuple[str, int],
        handler: type[BaseHTTPRequestHandler],
        role: str,
        state_dir: Path,
        fixtures: FixtureSet,
        defect: str | None,
        frame_origin: str | None,
    ) -> None:
        super().__init__(address, handler)
        self.role = role
        self.state_dir = state_dir
        self.fixtures = fixtures
        self.defect = defect
        self.frame_origin = frame_origin
        self.audit_lock = threading.Lock()
        self.asset_lock = threading.Lock()
        self.asset_seen = False
        self.stop_event = threading.Event()
        self.snapshot_lock = threading.Lock()
        self.snapshot_state = "not_configured"
        self.snapshot_ever_good = False
        self.poll_thread: threading.Thread | None = None

    @property
    def audit_path(self) -> Path:
        return self.state_dir / f"{self.role}.audit.jsonl"

    def audit(self, handler: BaseHTTPRequestHandler, route: str) -> None:
        user_agent = handler.headers.get("User-Agent", "")
        record = {
            "route": route,
            "method": handler.command,
            "cookie_names": cookie_names(handler.headers.get("Cookie")),
            "has_authorization": handler.headers.get("Authorization") is not None,
            "consumer": (
                "portfolio-background"
                if user_agent == "FramePortfolioPreview/1"
                else "browser-like-client"
            ),
        }
        if self.defect == "audit-sensitive-field":
            record["cookie_value"] = "synthetic-value-that-must-never-be-retained"
        encoded = json.dumps(record, sort_keys=True, separators=(",", ":")) + "\n"
        with self.audit_lock:
            with self.audit_path.open("a", encoding="utf-8") as output:
                output.write(encoded)
                output.flush()

    def start_portfolio_poller(self) -> None:
        require(
            self.role == "portfolio" and self.frame_origin is not None,
            "portfolio-startup",
            "portfolio poller configuration is invalid",
        )
        self.poll_thread = threading.Thread(
            target=self._poll_frame,
            name="portfolio-frame-public-poller",
            daemon=True,
        )
        self.poll_thread.start()

    def _poll_frame(self) -> None:
        assert self.frame_origin is not None
        opener = build_opener(ProxyHandler({}))
        cycle = 0
        while not self.stop_event.is_set():
            request = Request(
                f"{self.frame_origin}/api/v1/health",
                headers={
                    "Accept": "application/json",
                    "User-Agent": "FramePortfolioPreview/1",
                    "Connection": "close",
                },
            )
            succeeded = False
            try:
                with opener.open(request, timeout=PORTFOLIO_POLL_TIMEOUT_SECONDS) as response:
                    body = response.read(MAX_RESPONSE_BYTES + 1)
                    content_type = response.headers.get("Content-Type", "").split(";", 1)[0]
                    value = json.loads(body)
                    capabilities = value.get("capabilities") if isinstance(value, dict) else None
                    release = value.get("release") if isinstance(value, dict) else None
                    succeeded = (
                        response.status == 200
                        and len(body) <= MAX_RESPONSE_BYTES
                        and content_type == "application/json"
                        and value.get("api_version") == {"major": 1}
                        and value.get("service") == "frame"
                        and value.get("status") in {"ok", "degraded", "maintenance"}
                        and isinstance(release, str)
                        and SAFE_RELEASE_RE.fullmatch(release) is not None
                        and isinstance(capabilities, list)
                        and all(isinstance(item, str) for item in capabilities)
                    )
            except (
                HTTPError,
                URLError,
                TimeoutError,
                OSError,
                UnicodeDecodeError,
                json.JSONDecodeError,
            ):
                succeeded = False
            with self.snapshot_lock:
                if succeeded:
                    self.snapshot_state = "available"
                    self.snapshot_ever_good = True
                else:
                    self.snapshot_state = (
                        "stale" if self.snapshot_ever_good else "unavailable"
                    )
            cycle += 1
            interval = (0.050, 0.065, 0.080)[cycle % 3]
            self.stop_event.wait(interval)

    def stop_background(self) -> None:
        self.stop_event.set()
        if self.poll_thread is not None:
            self.poll_thread.join(timeout=1)


class QuietHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "FrameLocalSemanticFake/1"
    sys_version = ""
    server: PreviewHTTPServer

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def _send(
        self,
        status: int,
        body: bytes,
        headers: dict[str, str],
        head_only: bool = False,
    ) -> None:
        self.send_response(status)
        for name, value in headers.items():
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        if not head_only and body:
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                # Expected when the bounded portfolio poll abandons a slow fake.
                return

    def _api(
        self,
        status: int,
        body: bytes,
        content_type: str = "application/json; charset=utf-8",
        extra: dict[str, str] | None = None,
        head_only: bool = False,
    ) -> None:
        headers = dict(self.server.fixtures.contract["api_no_store_headers"])
        headers["content-type"] = content_type
        headers["x-request-id"] = "req_local_preview_01"
        if extra:
            headers.update(extra)
        self._send(status, body, headers, head_only=head_only)


class FrameHandler(QuietHandler):
    def do_GET(self) -> None:  # noqa: N802 - stdlib handler interface
        self._dispatch(False)

    def do_HEAD(self) -> None:  # noqa: N802 - stdlib handler interface
        self._dispatch(True)

    def do_POST(self) -> None:  # noqa: N802 - stdlib handler interface
        path = urlsplit(self.path).path
        route = {
            "/__local/session": "frame_session",
            "/logout": "frame_logout",
        }.get(path, "unknown")
        self.server.audit(self, route)
        if path == "/logout":
            self._api(
                HTTPStatus.NO_CONTENT,
                b"",
                extra={
                    "set-cookie": (
                        "frame_session=; Path=/; HttpOnly; SameSite=Lax; "
                        "Max-Age=0"
                    )
                },
            )
            return
        if path != "/__local/session":
            self._api(HTTPStatus.NOT_FOUND, self._generic_error("not_found", "never"))
            return
        cookie = "frame_session=frame-local-opaque; Path=/; HttpOnly; SameSite=Lax"
        if self.server.defect == "shared-cookie-domain":
            cookie += "; Domain=localhost"
        self._api(
            HTTPStatus.NO_CONTENT,
            b"",
            content_type="application/json; charset=utf-8",
            extra={"set-cookie": cookie},
        )

    def _dispatch(self, head_only: bool) -> None:
        path = urlsplit(self.path).path
        if path == "/__local/ready":
            self.server.audit(self, "frame_ready")
            self._api(
                HTTPStatus.OK,
                b'{"role":"frame","simulated":true}\n',
                head_only=head_only,
            )
            return
        if path == "/":
            self.server.audit(self, "frame_landing")
            body = (
                "<!doctype html><html lang=\"en\"><head>"
                "<meta charset=\"utf-8\"><title>Frame local preview</title>"
                "<meta name=\"robots\" content=\"noindex,nofollow\"></head>"
                "<body><main><h1>Frame</h1><p>Local semantic preview.</p>"
                "<a href=\"/login\">Sign in on Frame</a></main></body></html>"
            ).encode()
            self._send(
                HTTPStatus.OK,
                body,
                {
                    "content-type": "text/html; charset=utf-8",
                    "cache-control": "public, max-age=30",
                    "content-security-policy": "default-src 'self'; frame-ancestors 'none'",
                    "referrer-policy": "no-referrer",
                    "x-frame-options": "DENY",
                    "x-robots-tag": "noindex, nofollow, noarchive",
                },
                head_only,
            )
            return
        if path == "/login":
            self.server.audit(self, "frame_login")
            body = (
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">"
                "<title>Sign in to Frame</title>"
                "<meta name=\"robots\" content=\"noindex,nofollow\"></head>"
                "<body><main><h1>Sign in to Frame</h1>"
                "<p>Authentication remains on the Frame origin.</p></main></body></html>"
            ).encode()
            self._send(
                HTTPStatus.OK,
                body,
                {
                    "content-type": "text/html; charset=utf-8",
                    "cache-control": "no-store, max-age=0",
                    "content-security-policy": (
                        "default-src 'self'; base-uri 'none'; frame-ancestors 'none'"
                    ),
                    "referrer-policy": "no-referrer",
                    "x-frame-options": "DENY",
                    "x-robots-tag": "noindex, nofollow, noarchive",
                },
                head_only,
            )
            return
        if path == "/dashboard":
            self.server.audit(self, "frame_dashboard")
            if "frame_session" not in cookie_names(self.headers.get("Cookie")):
                self._api(
                    HTTPStatus.UNAUTHORIZED,
                    self._generic_error("unauthenticated", "never"),
                    extra={"www-authenticate": 'Session realm="frame"'},
                    head_only=head_only,
                )
                return
            self._api(
                HTTPStatus.OK,
                b'{"account":"synthetic-local","status":"ready"}\n',
                head_only=head_only,
            )
            return
        if path == "/api/v1/health":
            self.server.audit(self, "frame_health")
            self._health(head_only)
            return
        if path == "/api/v1/__local/error":
            self.server.audit(self, "frame_error")
            self._api(
                HTTPStatus.SERVICE_UNAVAILABLE,
                self.server.fixtures.json_bytes["error.json"],
                head_only=head_only,
            )
            return
        if path == "/api/v1/private/me":
            self.server.audit(self, "frame_private")
            extra = {"www-authenticate": 'Bearer realm="frame"'}
            if self.server.defect == "private-cache-hit":
                extra.update(
                    {
                        "cache-control": "public, max-age=300",
                        "x-frame-cache": "HIT",
                        "age": "99",
                    }
                )
            self._api(
                HTTPStatus.UNAUTHORIZED,
                self._generic_error("unauthenticated", "never"),
                extra=extra,
                head_only=head_only,
            )
            return
        if path == "/assets/frame.0123456789abcdef.js":
            self.server.audit(self, "frame_asset")
            body = b"globalThis.FRAME_CROSS_REPO_PREVIEW=true;\n"
            with self.server.asset_lock:
                cache_state = "HIT" if self.server.asset_seen else "MISS"
                self.server.asset_seen = True
            self._send(
                HTTPStatus.OK,
                body,
                {
                    "content-type": "text/javascript; charset=utf-8",
                    "cache-control": self.server.fixtures.contract[
                        "fingerprinted_asset_cache_control"
                    ],
                    "etag": '"frame-preview-asset-v1"',
                    "vary": "Accept-Encoding",
                    "x-frame-cache": cache_state,
                    "age": "1" if cache_state == "HIT" else "0",
                    "x-content-type-options": "nosniff",
                },
                head_only,
            )
            return
        share_prefix = "/api/v1/public/shares/"
        if path.startswith(share_prefix):
            remainder = path.removeprefix(share_prefix)
            if remainder.endswith("/media"):
                share_id = remainder.removesuffix("/media")
                self.server.audit(self, "frame_public_media")
                self._media(share_id, head_only)
                return
            if remainder.endswith("/captions/en"):
                share_id = remainder.removesuffix("/captions/en")
                self.server.audit(self, "frame_public_captions")
                self._captions(share_id, head_only)
                return
            self.server.audit(self, "frame_public_share")
            self._share(remainder, head_only)
            return
        self.server.audit(self, "frame_unknown")
        self._api(
            HTTPStatus.NOT_FOUND,
            self._generic_error("not_found", "never"),
            head_only=head_only,
        )

    def _health(self, head_only: bool) -> None:
        mode = (self.server.state_dir / "frame-mode").read_text(encoding="utf-8").strip()
        if mode == "slow":
            time.sleep(FRAME_SLOW_SECONDS)
            mode = "healthy"
        if mode == "healthy":
            self._api(
                HTTPStatus.OK,
                self.server.fixtures.json_bytes["health.ok.json"],
                head_only=head_only,
            )
            return
        if mode == "malformed":
            self._api(HTTPStatus.OK, b'{"api_version":', head_only=head_only)
            return
        if mode == "incompatible":
            body = json.dumps(
                {
                    "api_version": {"major": 2},
                    "service": "frame",
                    "status": "ok",
                    "release": "incompatible-local",
                    "capabilities": [],
                },
                separators=(",", ":"),
            ).encode()
            self._api(HTTPStatus.OK, body, head_only=head_only)
            return
        if mode == "error":
            self._api(
                HTTPStatus.SERVICE_UNAVAILABLE,
                self.server.fixtures.json_bytes["error.json"],
                head_only=head_only,
            )
            return
        self._api(
            HTTPStatus.INTERNAL_SERVER_ERROR,
            self._generic_error("invalid_fake_state", "never"),
            head_only=head_only,
        )

    def _share(self, share_id: str, head_only: bool) -> None:
        fixtures = self.server.fixtures.json_bytes
        if share_id == "public-demo":
            body = fixtures["share.public.json"]
        elif share_id == "processing-demo":
            body = fixtures["share.processing.json"]
        elif share_id == "privacy-demo":
            privacy = (self.server.state_dir / "privacy-mode").read_text(
                encoding="utf-8"
            ).strip()
            if privacy == "public":
                body = fixtures["share.public.json"].replace(b"public-demo", b"privacy-demo")
            else:
                body = fixtures["share.unavailable.json"]
        elif share_id in {
            "private-demo",
            "deleted-demo",
            "failed-demo",
            "unavailable-demo",
            "missing-demo",
        }:
            body = fixtures["share.unavailable.json"]
            if (
                share_id == "private-demo"
                and self.server.defect == "unavailable-title-leak"
            ):
                value = json.loads(body)
                value["title"] = "Private title must not cross the contract"
                body = json.dumps(value, separators=(",", ":")).encode()
        else:
            body = fixtures["share.unavailable.json"]
        self._api(HTTPStatus.OK, body, head_only=head_only)

    def _media(self, share_id: str, head_only: bool) -> None:
        if share_id not in {"public-demo", "privacy-demo"}:
            self._api(
                HTTPStatus.NOT_FOUND,
                self._generic_error("not_found", "never"),
                head_only=head_only,
            )
            return
        if share_id == "privacy-demo":
            privacy = (self.server.state_dir / "privacy-mode").read_text(
                encoding="utf-8"
            ).strip()
            if privacy != "public":
                self._api(
                    HTTPStatus.NOT_FOUND,
                    self._generic_error("not_found", "never"),
                    head_only=head_only,
                )
                return

        body = self.server.fixtures.media
        start = 0
        end = len(body) - 1
        status = HTTPStatus.OK
        range_header = self.headers.get("Range")
        extra = {
            "accept-ranges": "bytes",
            "content-disposition": "inline",
            "etag": f'"sha256-{hashlib.sha256(body).hexdigest()[:24]}"',
        }
        if range_header is not None:
            parsed = RANGE_RE.fullmatch(range_header)
            if parsed is None or "," in range_header:
                self._unsatisfied_range(len(body), head_only)
                return
            first, last = parsed.groups()
            try:
                if not first:
                    suffix = int(last)
                    if suffix <= 0:
                        raise ValueError
                    start = max(0, len(body) - suffix)
                else:
                    start = int(first)
                    end = len(body) - 1 if not last else min(int(last), len(body) - 1)
                    if start >= len(body) or end < start:
                        raise ValueError
            except ValueError:
                self._unsatisfied_range(len(body), head_only)
                return
            status = HTTPStatus.PARTIAL_CONTENT
            reported_end = end
            if self.server.defect == "range-off-by-one":
                reported_end = min(len(body), end + 1)
            extra["content-range"] = f"bytes {start}-{reported_end}/{len(body)}"
            body = body[start : end + 1]
        self._api(
            status,
            body,
            content_type="video/mp4",
            extra=extra,
            head_only=head_only,
        )

    def _unsatisfied_range(self, size: int, head_only: bool) -> None:
        self._api(
            HTTPStatus.REQUESTED_RANGE_NOT_SATISFIABLE,
            self._generic_error("range_not_satisfiable", "never"),
            extra={"content-range": f"bytes */{size}"},
            head_only=head_only,
        )

    def _captions(self, share_id: str, head_only: bool) -> None:
        if share_id not in {"public-demo", "privacy-demo"}:
            self._api(
                HTTPStatus.NOT_FOUND,
                self._generic_error("not_found", "never"),
                head_only=head_only,
            )
            return
        if share_id == "privacy-demo" and (
            self.server.state_dir / "privacy-mode"
        ).read_text(encoding="utf-8").strip() != "public":
            self._api(
                HTTPStatus.NOT_FOUND,
                self._generic_error("not_found", "never"),
                head_only=head_only,
            )
            return
        self._api(
            HTTPStatus.OK,
            self.server.fixtures.captions,
            content_type="text/vtt; charset=utf-8",
            extra={"content-language": "en"},
            head_only=head_only,
        )

    @staticmethod
    def _generic_error(code: str, retry: str) -> bytes:
        return json.dumps(
            {
                "code": code,
                "message": "The requested resource is unavailable.",
                "request_id": "req_local_preview_01",
                "retry": retry,
            },
            separators=(",", ":"),
        ).encode()


class PortfolioHandler(QuietHandler):
    def do_GET(self) -> None:  # noqa: N802 - stdlib handler interface
        path = urlsplit(self.path).path
        if path == "/__local/ready":
            self.server.audit(self, "portfolio_ready")
            self._api(
                HTTPStatus.OK,
                b'{"role":"portfolio","simulated":true}\n',
            )
            return
        if path == "/":
            self.server.audit(self, "portfolio_home")
            self._homepage()
            return
        if path == "/assets/portfolio.0123456789abcdef.css":
            self.server.audit(self, "portfolio_asset")
            self._send(
                HTTPStatus.OK,
                b"body{font-family:system-ui}\n",
                {
                    "content-type": "text/css; charset=utf-8",
                    "cache-control": self.server.fixtures.contract[
                        "fingerprinted_asset_cache_control"
                    ],
                    "etag": '"portfolio-preview-asset-v1"',
                    "vary": "Accept-Encoding",
                    "x-content-type-options": "nosniff",
                },
            )
            return
        self.server.audit(self, "portfolio_unknown")
        self._send(
            HTTPStatus.NOT_FOUND,
            b"Not found.\n",
            {
                "content-type": "text/plain; charset=utf-8",
                "cache-control": "no-store, max-age=0",
            },
        )

    def do_POST(self) -> None:  # noqa: N802 - stdlib handler interface
        path = urlsplit(self.path).path
        self.server.audit(
            self,
            "portfolio_session" if path == "/__local/session" else "portfolio_unknown",
        )
        if path != "/__local/session":
            self._send(
                HTTPStatus.NOT_FOUND,
                b"",
                {"cache-control": "no-store, max-age=0"},
            )
            return
        self._send(
            HTTPStatus.NO_CONTENT,
            b"",
            {
                "cache-control": "no-store, max-age=0",
                "set-cookie": (
                    "portfolio_session=portfolio-local-opaque; Path=/; "
                    "HttpOnly; SameSite=Lax"
                ),
            },
        )

    def _homepage(self) -> None:
        assert self.server.frame_origin is not None
        if self.server.defect == "handler-path-upstream-fetch":
            request = Request(
                f"{self.server.frame_origin}/api/v1/health",
                headers={"Connection": "close", "User-Agent": "DefectiveHandler/1"},
            )
            try:
                build_opener(ProxyHandler({})).open(request, timeout=0.600).read(1)
            except (HTTPError, URLError, TimeoutError, OSError):
                pass

        with self.server.snapshot_lock:
            state = self.server.snapshot_state
        labels = {
            "not_configured": "Frame link ready",
            "available": "Frame available",
            "stale": "Frame status temporarily stale",
            "unavailable": "Frame status unavailable",
        }
        label = labels.get(state, "Frame status unavailable")
        origin = html.escape(self.server.frame_origin, quote=True)
        body = (
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">"
            "<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">"
            "<title>EngManager local preview</title>"
            "<meta name=\"robots\" content=\"noindex,nofollow\">"
            "<link rel=\"stylesheet\" href=\"/assets/portfolio.0123456789abcdef.css\">"
            "</head><body><main><h1>EngManager</h1>"
            f"<article data-frame-state=\"{state}\"><h2>Frame</h2>"
            f"<p aria-live=\"polite\">{label}</p>"
            f"<a id=\"frame-link\" aria-label=\"Open Frame recording workspace\" "
            f"href=\"{origin}/\">Open Frame</a></article>"
            "<section><h2>Portfolio content</h2><p>Always available locally.</p>"
            "</section></main></body></html>"
        ).encode()
        self._send(
            HTTPStatus.OK,
            body,
            {
                "content-type": "text/html; charset=utf-8",
                "cache-control": self.server.fixtures.contract[
                    "portfolio_public_cache_control"
                ],
                "vary": "Accept-Encoding",
                "content-security-policy": (
                    "default-src 'self'; style-src 'self'; connect-src 'self'; "
                    "frame-src 'none'; frame-ancestors 'none'; base-uri 'none'"
                ),
                "permissions-policy": (
                    "camera=(), microphone=(), display-capture=(), autoplay=()"
                ),
                "referrer-policy": "strict-origin-when-cross-origin",
                "x-frame-options": "DENY",
                "x-content-type-options": "nosniff",
                "x-robots-tag": "noindex, nofollow, noarchive",
            },
        )


@dataclass(frozen=True)
class HttpResult:
    status: int
    headers: dict[str, str]
    body: bytes

    def json(self, check: str) -> dict[str, Any]:
        try:
            value = json.loads(self.body)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise HarnessFailure(check, "response is not valid JSON") from error
        require(isinstance(value, dict), check, "response JSON is not an object")
        return value


def http_request(
    method: str,
    url: str,
    *,
    headers: dict[str, str] | None = None,
    data: bytes | None = None,
    opener: Any | None = None,
    timeout: float = 1.5,
) -> HttpResult:
    request_headers = {"Connection": "close"}
    if headers:
        request_headers.update(headers)
    request = Request(url, data=data, headers=request_headers, method=method)
    client = opener or build_opener(ProxyHandler({}))
    try:
        response = client.open(request, timeout=timeout)
    except HTTPError as error:
        response = error
    except (URLError, TimeoutError, OSError) as error:
        raise HarnessFailure("http-request", "loopback request failed") from error
    with response:
        body = response.read(MAX_RESPONSE_BYTES + 1)
        require(
            len(body) <= MAX_RESPONSE_BYTES,
            "http-response-bound",
            "loopback response exceeded the harness byte limit",
        )
        normalized = {name.lower(): value for name, value in response.headers.items()}
        return HttpResult(int(response.status), normalized, body)


class FrameLinkParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.href: str | None = None
        self.aria_label: str | None = None

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        values = dict(attrs)
        if tag == "a" and values.get("id") == "frame-link":
            self.href = values.get("href")
            self.aria_label = values.get("aria-label")


class ManagedServer:
    def __init__(
        self,
        role: str,
        state_dir: Path,
        defect: str | None,
        frame_origin: str | None = None,
        listen_port: int = 0,
    ) -> None:
        self.role = role
        self.state_dir = state_dir
        self.port_path = state_dir / f"{role}.port"
        self.log_path = state_dir / f"{role}.log"
        self.log_handle = self.log_path.open("wb")
        self.port_path.unlink(missing_ok=True)
        command = [
            sys.executable,
            "-I",
            str(Path(__file__).resolve()),
            "--serve",
            role,
            "--port",
            str(listen_port),
            "--state-dir",
            str(state_dir),
        ]
        if defect:
            command.extend(["--defect", defect])
        if frame_origin:
            command.extend(["--frame-origin", frame_origin])
        self.process = subprocess.Popen(
            command,
            cwd=ROOT,
            env=child_environment(),
            stdin=subprocess.DEVNULL,
            stdout=self.log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        self.port: int | None = None

    def await_origin(self, host: str, timeout: float) -> str:
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if self.process.poll() is not None:
                self.log_handle.flush()
                try:
                    detail = self.log_path.read_text(
                        encoding="utf-8", errors="replace"
                    ).strip()
                except OSError:
                    detail = ""
                suffix = f": {detail[-2_000:]}" if detail else ""
                raise HarnessFailure(
                    f"{self.role}-startup",
                    f"{self.role} semantic fake exited during startup{suffix}",
                )
            if self.port_path.is_file():
                try:
                    value = int(self.port_path.read_text(encoding="ascii").strip())
                except (OSError, UnicodeDecodeError, ValueError):
                    value = 0
                if 1 <= value <= 65535:
                    self.port = value
                    origin = f"http://{host}:{value}"
                    try:
                        result = http_request("GET", f"{origin}/__local/ready", timeout=0.3)
                    except HarnessFailure:
                        time.sleep(0.02)
                        continue
                    if result.status == 200:
                        return origin
            time.sleep(0.02)
        self.log_handle.flush()
        try:
            detail = self.log_path.read_text(encoding="utf-8", errors="replace").strip()
        except OSError:
            detail = ""
        suffix = f": {detail[-2_000:]}" if detail else ""
        raise HarnessFailure(
            f"{self.role}-startup",
            f"{self.role} semantic fake did not become ready{suffix}",
        )

    def stop(self) -> None:
        if self.process.poll() is None:
            if os.name == "posix":
                try:
                    os.killpg(self.process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            else:
                self.process.terminate()
            try:
                self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                if os.name == "posix":
                    try:
                        os.killpg(self.process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                else:
                    self.process.kill()
                self.process.wait(timeout=1)
        if not self.log_handle.closed:
            self.log_handle.close()

    def prove_closed(self) -> None:
        if self.port is None:
            return
        end = time.monotonic() + 2
        while time.monotonic() < end:
            try:
                connection = socket.create_connection(("127.0.0.1", self.port), timeout=0.05)
            except OSError:
                return
            connection.close()
            time.sleep(0.03)
        raise HarnessFailure("deterministic-cleanup", f"{self.role} listener remained open")


def run_server(args: argparse.Namespace) -> int:
    state_dir = Path(args.state_dir).resolve()
    require(state_dir.is_dir(), "server-state", "server state directory is missing")
    fixtures = load_fixtures()
    handler: type[BaseHTTPRequestHandler]
    if args.serve == "frame":
        handler = FrameHandler
    else:
        handler = PortfolioHandler
        parsed = urlsplit(args.frame_origin or "")
        require(
            parsed.scheme == "http"
            and parsed.hostname == "127.0.0.1"
            and parsed.port is not None
            and parsed.path in {"", "/"}
            and not parsed.query
            and not parsed.fragment
            and not parsed.username
            and not parsed.password,
            "portfolio-origin",
            "portfolio fake requires an explicit loopback Frame origin",
        )
    server = PreviewHTTPServer(
        ("127.0.0.1", args.port),
        handler,
        args.serve,
        state_dir,
        fixtures,
        args.defect,
        args.frame_origin,
    )
    atomic_write(state_dir / f"{args.serve}.port", f"{server.server_port}\n")
    if args.serve == "portfolio":
        server.start_portfolio_poller()
    try:
        server.serve_forever(poll_interval=0.05)
    finally:
        server.stop_background()
        server.server_close()
    return 0


def assert_no_store(result: HttpResult, fixtures: FixtureSet, check: str) -> None:
    for name, expected in fixtures.contract["api_no_store_headers"].items():
        require(
            result.headers.get(name) == expected,
            check,
            f"API response header {name} drifted",
        )
    require(
        result.headers.get("x-request-id") == "req_local_preview_01",
        check,
        "API request identifier policy drifted",
    )
    require(
        "access-control-allow-origin" not in result.headers,
        check,
        "direct portfolio browser CORS was enabled unexpectedly",
    )
    require(
        result.headers.get("x-frame-cache") != "HIT" and "age" not in result.headers,
        check,
        "private/dynamic API response appeared cacheable or cached",
    )


def assert_fixture_response(
    result: HttpResult,
    fixtures: FixtureSet,
    fixture_name: str,
    status: int,
    check: str,
) -> None:
    require(result.status == status, check, f"{fixture_name} status drifted")
    require(
        result.headers.get("content-type") == "application/json; charset=utf-8",
        check,
        f"{fixture_name} content type drifted",
    )
    assert_no_store(result, fixtures, check)
    require(
        result.body == fixtures.json_bytes[fixture_name],
        check,
        f"{fixture_name} response bytes drifted",
    )


def read_audit(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    for _attempt in range(4):
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
            return [json.loads(line) for line in lines if line]
        except (OSError, UnicodeDecodeError, json.JSONDecodeError):
            time.sleep(0.02)
    raise HarnessFailure("audit-integrity", "semantic audit stream was not valid JSONL")


def wait_for(
    predicate: Callable[[], bool],
    *,
    timeout: float,
    check: str,
    message: str,
) -> None:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if predicate():
            return
        time.sleep(0.025)
    raise HarnessFailure(check, message)


def timed_home(portfolio_origin: str, opener: Any) -> tuple[HttpResult, float]:
    started = time.monotonic()
    result = http_request("GET", f"{portfolio_origin}/", opener=opener)
    return result, time.monotonic() - started


def home_has_state(portfolio_origin: str, state: str) -> bool:
    try:
        result = http_request("GET", f"{portfolio_origin}/", timeout=0.4)
    except HarnessFailure:
        return False
    return result.status == 200 and f'data-frame-state="{state}"'.encode() in result.body


def check_cookie_header(result: HttpResult, check: str) -> None:
    value = result.headers.get("set-cookie", "")
    require(value and "Domain=" not in value, check, "session cookie is not host-only")
    require(
        "HttpOnly" in value and "SameSite=Lax" in value and "Path=/" in value,
        check,
        "local host-only cookie attributes drifted",
    )


def run_case(defect: str | None, timeout: float, report: bool) -> None:
    fixtures = load_fixtures()
    frame: ManagedServer | None = None
    portfolio: ManagedServer | None = None
    pending_failure: BaseException | None = None

    with tempfile.TemporaryDirectory(prefix="frame-cross-repo-preview-") as temporary:
        state_dir = Path(temporary)
        atomic_write(state_dir / "frame-mode", "healthy\n")
        atomic_write(state_dir / "privacy-mode", "public\n")
        try:
            frame = ManagedServer("frame", state_dir, defect)
            frame_origin = frame.await_origin("127.0.0.1", min(timeout, 10))
            portfolio = ManagedServer(
                "portfolio", state_dir, defect, frame_origin=frame_origin
            )
            portfolio_origin = portfolio.await_origin("localhost", min(timeout, 10))
            require(
                urlsplit(frame_origin).hostname != urlsplit(portfolio_origin).hostname
                and frame_origin != portfolio_origin,
                "origin-isolation",
                "portfolio and Frame did not receive distinct loopback origins",
            )

            wait_for(
                lambda: home_has_state(portfolio_origin, "available"),
                timeout=min(timeout, 3),
                check="portfolio-background-poll",
                message="bounded public background poll did not publish healthy state",
            )
            emit(report, "two distinct loopback origins and bounded background polling")

            health = http_request("GET", f"{frame_origin}/api/v1/health")
            assert_fixture_response(
                health, fixtures, "health.ok.json", 200, "health-contract"
            )
            public_share = http_request(
                "GET", f"{frame_origin}/api/v1/public/shares/public-demo"
            )
            assert_fixture_response(
                public_share,
                fixtures,
                "share.public.json",
                200,
                "share-contract",
            )
            processing_share = http_request(
                "GET", f"{frame_origin}/api/v1/public/shares/processing-demo"
            )
            assert_fixture_response(
                processing_share,
                fixtures,
                "share.processing.json",
                200,
                "share-contract",
            )
            error = http_request("GET", f"{frame_origin}/api/v1/__local/error")
            assert_fixture_response(
                error,
                fixtures,
                "error.json",
                503,
                "error-contract",
            )
            emit(
                report,
                "exact health, public/processing-share, error, and no-store HTTP contracts",
            )

            jar = http.cookiejar.CookieJar()
            browser = build_opener(ProxyHandler({}), HTTPCookieProcessor(jar))
            portfolio_session = http_request(
                "POST",
                f"{portfolio_origin}/__local/session",
                data=b"",
                opener=browser,
            )
            require(
                portfolio_session.status == 204,
                "portfolio-host-only-cookie",
                "portfolio session setup failed",
            )
            check_cookie_header(portfolio_session, "portfolio-host-only-cookie")
            portfolio_home = http_request(
                "GET", f"{portfolio_origin}/", opener=browser
            )
            parser = FrameLinkParser()
            parser.feed(portfolio_home.body.decode("utf-8"))
            require(
                parser.href == f"{frame_origin}/"
                and parser.aria_label == "Open Frame recording workspace",
                "top-level-navigation",
                "accessible canonical Frame link is missing or ambiguous",
            )
            link = urlsplit(parser.href or "")
            require(
                not link.query
                and not link.fragment
                and not link.username
                and not link.password,
                "top-level-navigation",
                "top-level Frame navigation carried credentials or return state",
            )
            frame_landing = http_request("GET", parser.href or "", opener=browser)
            require(
                frame_landing.status == 200 and b"<h1>Frame</h1>" in frame_landing.body,
                "top-level-navigation",
                "Frame landing did not render anonymously",
            )
            landing_records = [
                record
                for record in read_audit(state_dir / "frame.audit.jsonl")
                if record["route"] == "frame_landing"
            ]
            require(
                landing_records
                and landing_records[-1]["cookie_names"] == []
                and landing_records[-1]["has_authorization"] is False,
                "top-level-navigation",
                "portfolio authority crossed the top-level navigation boundary",
            )
            login = http_request("GET", f"{frame_origin}/login", opener=browser)
            denied_dashboard = http_request(
                "GET", f"{frame_origin}/dashboard", opener=browser
            )
            require(
                login.status == 200
                and b"Sign in to Frame" in login.body
                and login.headers.get("x-robots-tag") == "noindex, nofollow, noarchive",
                "login-boundary",
                "anonymous Frame login boundary drifted",
            )
            require(
                denied_dashboard.status == 401,
                "dashboard-boundary",
                "anonymous dashboard request was not denied",
            )
            assert_no_store(denied_dashboard, fixtures, "dashboard-boundary")

            frame_session = http_request(
                "POST",
                f"{frame_origin}/__local/session",
                data=b"",
                opener=browser,
            )
            require(
                frame_session.status == 204,
                "frame-host-only-cookie",
                "Frame session setup failed",
            )
            check_cookie_header(frame_session, "frame-host-only-cookie")
            http_request("GET", f"{frame_origin}/", opener=browser)
            http_request("GET", f"{portfolio_origin}/", opener=browser)
            frame_landing_records = [
                record
                for record in read_audit(state_dir / "frame.audit.jsonl")
                if record["route"] == "frame_landing"
            ]
            portfolio_home_records = [
                record
                for record in read_audit(state_dir / "portfolio.audit.jsonl")
                if record["route"] == "portfolio_home"
            ]
            require(
                frame_landing_records[-1]["cookie_names"] == ["frame_session"]
                and portfolio_home_records[-1]["cookie_names"]
                == ["portfolio_session"],
                "cookie-origin-isolation",
                "host-local sessions crossed between loopback origins",
            )
            dashboard = http_request("GET", f"{frame_origin}/dashboard", opener=browser)
            require(
                dashboard.status == 200
                and dashboard.json("dashboard-boundary")
                == {"account": "synthetic-local", "status": "ready"},
                "dashboard-boundary",
                "host-local Frame session did not enter the synthetic dashboard",
            )
            assert_no_store(dashboard, fixtures, "dashboard-boundary")
            logout = http_request(
                "POST", f"{frame_origin}/logout", data=b"", opener=browser
            )
            require(
                logout.status == 204
                and "Max-Age=0" in logout.headers.get("set-cookie", ""),
                "logout-boundary",
                "Frame logout did not revoke the host-local session",
            )
            assert_no_store(logout, fixtures, "logout-boundary")
            denied_after_logout = http_request(
                "GET", f"{frame_origin}/dashboard", opener=browser
            )
            require(
                denied_after_logout.status == 401,
                "logout-boundary",
                "dashboard remained available after logout",
            )
            assert_no_store(denied_after_logout, fixtures, "logout-boundary")
            emit(
                report,
                "top-level navigation, Back-equivalent return, login/dashboard/logout, and host-only cookies",
            )

            for _attempt in range(3):
                private = http_request(
                    "GET", f"{frame_origin}/api/v1/private/me", opener=browser
                )
                require(
                    private.status == 401
                    and private.headers.get("cache-control") == "no-store, max-age=0"
                    and private.headers.get("x-frame-cache") != "HIT"
                    and "age" not in private.headers,
                    "private-cache-isolation",
                    "private/auth response was cacheable or appeared as a cache hit",
                )
                assert_no_store(private, fixtures, "private-cache-isolation")
            asset_path = "/assets/frame.0123456789abcdef.js"
            asset_first = http_request("GET", f"{frame_origin}{asset_path}")
            asset_second = http_request("GET", f"{frame_origin}{asset_path}")
            require(
                asset_first.status == asset_second.status == 200
                and asset_first.body == asset_second.body
                and asset_first.headers.get("x-frame-cache") == "MISS"
                and asset_second.headers.get("x-frame-cache") == "HIT"
                and asset_second.headers.get("cache-control")
                == fixtures.contract["fingerprinted_asset_cache_control"]
                and asset_second.headers.get("vary") == "Accept-Encoding"
                and "set-cookie" not in asset_second.headers,
                "asset-cache-contract",
                "fingerprinted public asset did not transition from MISS to HIT",
            )
            emit(report, "private/dynamic bypass and isolated fingerprinted-asset HIT")

            full_head = http_request(
                "HEAD", f"{frame_origin}/api/v1/public/shares/public-demo/media"
            )
            require(
                full_head.status == 200
                and full_head.body == b""
                and full_head.headers.get("content-length") == str(len(fixtures.media))
                and full_head.headers.get("accept-ranges") == "bytes",
                "range-contract",
                "public media HEAD contract drifted",
            )
            assert_no_store(full_head, fixtures, "range-contract")
            byte_range = http_request(
                "GET",
                f"{frame_origin}/api/v1/public/shares/public-demo/media",
                headers={"Range": "bytes=10-19"},
            )
            require(
                byte_range.status == 206
                and byte_range.body == fixtures.media[10:20]
                and byte_range.headers.get("content-range")
                == f"bytes 10-19/{len(fixtures.media)}"
                and byte_range.headers.get("content-length") == "10"
                and byte_range.headers.get("content-type") == "video/mp4",
                "range-contract",
                "bounded public media Range response drifted",
            )
            assert_no_store(byte_range, fixtures, "range-contract")
            suffix = http_request(
                "GET",
                f"{frame_origin}/api/v1/public/shares/public-demo/media",
                headers={"Range": "bytes=-7"},
            )
            require(
                suffix.status == 206
                and suffix.body == fixtures.media[-7:]
                and suffix.headers.get("content-range")
                == f"bytes {len(fixtures.media) - 7}-{len(fixtures.media) - 1}/{len(fixtures.media)}",
                "range-contract",
                "suffix Range response drifted",
            )
            invalid_range = http_request(
                "GET",
                f"{frame_origin}/api/v1/public/shares/public-demo/media",
                headers={"Range": "bytes=999-1000"},
            )
            require(
                invalid_range.status == 416
                and invalid_range.headers.get("content-range")
                == f"bytes */{len(fixtures.media)}",
                "range-contract",
                "unsatisfied Range response drifted",
            )
            assert_no_store(invalid_range, fixtures, "range-contract")
            captions = http_request(
                "GET",
                f"{frame_origin}/api/v1/public/shares/public-demo/captions/en",
            )
            require(
                captions.status == 200
                and captions.body == fixtures.captions
                and captions.headers.get("content-type") == "text/vtt; charset=utf-8"
                and captions.headers.get("content-language") == "en",
                "caption-contract",
                "public caption response drifted",
            )
            assert_no_store(captions, fixtures, "caption-contract")

            unavailable_bodies: list[bytes] = []
            for share_id in (
                "private-demo",
                "deleted-demo",
                "failed-demo",
                "unavailable-demo",
                "missing-demo",
            ):
                response = http_request(
                    "GET", f"{frame_origin}/api/v1/public/shares/{share_id}"
                )
                require(
                    response.status == 200,
                    "unavailable-privacy",
                    "non-public share status exposed existence",
                )
                assert_no_store(response, fixtures, "unavailable-privacy")
                unavailable_bodies.append(response.body)
            require(
                all(
                    body == fixtures.json_bytes["share.unavailable.json"]
                    for body in unavailable_bodies
                ),
                "unavailable-privacy",
                "private/deleted/failed/missing states were distinguishable",
            )
            emit(report, "Range, captions, generic unavailable states, and privacy redaction")

            privacy_share = http_request(
                "GET", f"{frame_origin}/api/v1/public/shares/privacy-demo"
            )
            require(
                privacy_share.json("privacy-purge").get("availability") == "public",
                "privacy-purge",
                "privacy transition seed was not public",
            )
            privacy_media = http_request(
                "GET",
                f"{frame_origin}/api/v1/public/shares/privacy-demo/media",
                headers={"Range": "bytes=0-3"},
            )
            require(
                privacy_media.status == 206,
                "privacy-purge",
                "public privacy seed media was unavailable",
            )
            privacy_started = time.monotonic()
            atomic_write(state_dir / "privacy-mode", "private\n")
            hidden_share = http_request(
                "GET",
                f"{frame_origin}/api/v1/public/shares/privacy-demo",
                headers={"Cache-Control": "no-cache"},
            )
            hidden_media = http_request(
                "GET",
                f"{frame_origin}/api/v1/public/shares/privacy-demo/media",
                headers={"Range": "bytes=0-3", "Cache-Control": "no-cache"},
            )
            require(
                hidden_share.body == fixtures.json_bytes["share.unavailable.json"]
                and hidden_media.status == 404
                and hidden_media.headers.get("x-frame-cache") != "HIT"
                and time.monotonic() - privacy_started < 0.500,
                "privacy-purge",
                "privacy change did not revoke summary and media inside the local SLO",
            )
            assert_no_store(hidden_share, fixtures, "privacy-purge")
            assert_no_store(hidden_media, fixtures, "privacy-purge")
            emit(report, "immediate local privacy revocation with no stale cache replay")

            for mode in ("malformed", "incompatible", "error"):
                atomic_write(state_dir / "frame-mode", f"{mode}\n")
                wait_for(
                    lambda: home_has_state(portfolio_origin, "stale"),
                    timeout=min(timeout, 2),
                    check="portfolio-degradation",
                    message=f"portfolio did not degrade after {mode} Frame response",
                )
                page, latency = timed_home(portfolio_origin, browser)
                require(
                    page.status == 200
                    and latency < PORTFOLIO_HANDLER_BUDGET_SECONDS
                    and parser.href.encode() in page.body,
                    "portfolio-latency",
                    f"portfolio route coupled to {mode} Frame response",
                )

            atomic_write(state_dir / "frame-mode", "healthy\n")
            wait_for(
                lambda: home_has_state(portfolio_origin, "available"),
                timeout=min(timeout, 2),
                check="portfolio-retry-recovery",
                message="portfolio background retry did not recover after rollback",
            )

            health_before = len(
                [
                    record
                    for record in read_audit(state_dir / "frame.audit.jsonl")
                    if record["route"] == "frame_health"
                ]
            )
            atomic_write(state_dir / "frame-mode", "slow\n")
            wait_for(
                lambda: len(
                    [
                        record
                        for record in read_audit(state_dir / "frame.audit.jsonl")
                        if record["route"] == "frame_health"
                    ]
                )
                > health_before,
                timeout=min(timeout, 2),
                check="portfolio-degradation",
                message="slow Frame fault was not exercised",
            )
            slow_page, slow_latency = timed_home(portfolio_origin, browser)
            require(
                slow_page.status == 200
                and slow_latency < PORTFOLIO_HANDLER_BUDGET_SECONDS
                and b"Portfolio content" in slow_page.body,
                "portfolio-latency",
                "slow Frame response delayed the portfolio handler",
            )

            old_port = frame.port
            require(old_port is not None, "frame-restart", "Frame port was not recorded")
            frame.stop()
            frame.prove_closed()
            frame = None
            wait_for(
                lambda: home_has_state(portfolio_origin, "stale"),
                timeout=min(timeout, 2),
                check="portfolio-degradation",
                message="portfolio did not retain stale state during Frame outage",
            )
            outage_page, outage_latency = timed_home(portfolio_origin, browser)
            require(
                outage_page.status == 200
                and outage_latency < PORTFOLIO_HANDLER_BUDGET_SECONDS
                and b"Open Frame" in outage_page.body
                and b"Portfolio content" in outage_page.body,
                "portfolio-latency",
                "Frame outage delayed or removed normal portfolio content",
            )
            portfolio_asset = http_request(
                "GET", f"{portfolio_origin}/assets/portfolio.0123456789abcdef.css"
            )
            require(
                portfolio_asset.status == 200
                and portfolio_asset.headers.get("cache-control")
                == fixtures.contract["fingerprinted_asset_cache_control"],
                "portfolio-degradation",
                "portfolio static route failed during Frame outage",
            )
            forbidden_markers = (
                b"Public product walkthrough",
                b"portfolio-local-opaque",
                b"frame-local-opaque",
                b"Authorization",
                b"signed_url",
                b"tenant_id",
            )
            require(
                not any(marker in outage_page.body for marker in forbidden_markers),
                "portfolio-public-data",
                "portfolio HTML contained private or upstream response material",
            )
            atomic_write(state_dir / "frame-mode", "healthy\n")
            frame = ManagedServer(
                "frame", state_dir, defect, listen_port=old_port
            )
            restarted_origin = frame.await_origin("127.0.0.1", min(timeout, 4))
            require(
                restarted_origin == frame_origin,
                "frame-restart",
                "Frame semantic deploy did not retain the paired origin",
            )
            wait_for(
                lambda: home_has_state(portfolio_origin, "available"),
                timeout=min(timeout, 3),
                check="portfolio-retry-recovery",
                message="portfolio did not reconnect after Frame restart",
            )
            emit(
                report,
                "malformed/version/error/slow/outage plus rollback/retry/reconnect within latency budget",
            )

            frame_audit = read_audit(state_dir / "frame.audit.jsonl")
            portfolio_audit = read_audit(state_dir / "portfolio.audit.jsonl")
            safe_audit_fields = {
                "route",
                "method",
                "cookie_names",
                "has_authorization",
                "consumer",
            }
            require(
                all(set(record) == safe_audit_fields for record in frame_audit)
                and all(set(record) == safe_audit_fields for record in portfolio_audit),
                "audit-redaction",
                "audit retained a response, credential, identifier, or unapproved field",
            )
            background = [
                record
                for record in frame_audit
                if record["consumer"] == "portfolio-background"
            ]
            require(
                background
                and all(
                    record["cookie_names"] == []
                    and record["has_authorization"] is False
                    and record["route"] == "frame_health"
                    for record in background
                ),
                "background-authority",
                "portfolio background consumer sent ambient authority or a non-health request",
            )
            require(
                all(record["has_authorization"] is False for record in frame_audit)
                and all(
                    set(record["cookie_names"]) <= {"portfolio_session"}
                    and record["has_authorization"] is False
                    for record in portfolio_audit
                ),
                "origin-authority-isolation",
                "audit found a token or cross-origin cookie dependency",
            )
            emit(report, "redacted audits prove origin, cookie, token, and route isolation")
        except BaseException as error:  # preserve the first journey failure through cleanup
            pending_failure = error
        finally:
            if frame is not None:
                frame.stop()
            if portfolio is not None:
                portfolio.stop()
            try:
                if frame is not None:
                    frame.prove_closed()
                if portfolio is not None:
                    portfolio.prove_closed()
            except BaseException as cleanup_error:
                pending_failure = cleanup_error

        if pending_failure is not None:
            raise pending_failure
        emit(report, "process groups stopped and both loopback listeners closed")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run the credential-free local portfolio-to-Frame contract preview"
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="also prove every seeded contract defect is detected",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=20,
        help="per-case startup/fault deadline in seconds (default: 20)",
    )
    parser.add_argument("--serve", choices=("frame", "portfolio"), help=argparse.SUPPRESS)
    parser.add_argument("--port", type=int, default=0, help=argparse.SUPPRESS)
    parser.add_argument("--state-dir", help=argparse.SUPPRESS)
    parser.add_argument("--frame-origin", help=argparse.SUPPRESS)
    parser.add_argument(
        "--defect", choices=tuple(DEFECT_EXPECTATIONS), help=argparse.SUPPRESS
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.serve:
        require(args.state_dir is not None, "server-state", "missing server state")
        return run_server(args)
    require(
        2 <= args.timeout <= 120,
        "arguments",
        "timeout must be between 2 and 120 seconds",
    )
    refuse_provider_state()
    run_case(None, args.timeout, report=True)
    if args.self_test:
        for defect, expected_check in DEFECT_EXPECTATIONS.items():
            try:
                run_case(defect, args.timeout, report=False)
            except HarnessFailure as error:
                require(
                    error.check == expected_check,
                    "negative-control",
                    f"{defect} tripped {error.check} instead of {expected_check}",
                )
                print(f"[control] {defect}: detected by {expected_check}", flush=True)
            else:
                raise HarnessFailure(
                    "negative-control", f"{defect} was not detected by the journey"
                )
    print(
        "Cross-repository local semantic preview passed; no provider/browser compatibility claimed.",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except HarnessFailure as error:
        print(f"cross-repo preview failed [{error.check}]: {error}", file=sys.stderr)
        raise SystemExit(1) from None
