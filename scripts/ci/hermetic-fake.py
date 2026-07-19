#!/usr/bin/env python3
"""Loopback-only storage, media, cache, and state semantic fake.

This server is deliberately not a Cloudflare, browser, or media emulator.
Every dynamic response is labeled as simulated and it is only exercised by
the provider-free hermetic journey.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sqlite3
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit


MAX_BODY = 1024 * 1024
ASSET_PATH = "/assets/app.0123456789abcdef.js"
ASSET_BODY = b"globalThis.FRAME_HERMETIC_ASSET=true;\n"
RANGE_RE = re.compile(r"^bytes=(\d*)-(\d*)$")


class HermeticServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = False

    def __init__(
        self,
        address: tuple[str, int],
        state_dir: Path,
        fixture: dict[str, Any],
        web_origin: str,
        inject_private_cache_hit: bool,
    ) -> None:
        super().__init__(address, HermeticHandler)
        self.state_dir = state_dir
        self.fixture = fixture
        self.web_origin = web_origin
        self.inject_private_cache_hit = inject_private_cache_hit
        self.asset_seen = False
        self.media_cache: dict[str, bytes] = {}
        self.db_path = state_dir / "semantic-d1.sqlite3"
        (state_dir / "objects").mkdir(parents=True, exist_ok=True)
        initialize_database(self.db_path)


class HermeticHandler(BaseHTTPRequestHandler):
    server: HermeticServer
    protocol_version = "HTTP/1.1"
    server_version = "FrameHermetic/1"
    sys_version = ""

    def log_message(self, _format: str, *_args: object) -> None:
        # Request paths can contain identifiers. The orchestrator emits only
        # named checks, so raw HTTP traces never become CI artifacts.
        return

    def do_OPTIONS(self) -> None:  # noqa: N802 - stdlib handler contract
        path = urlsplit(self.path).path
        if path.startswith("/fake-r2/uploads/"):
            self.send_response(HTTPStatus.NO_CONTENT)
            self._cors_headers()
            self.send_header("Access-Control-Allow-Methods", "PUT, OPTIONS")
            self.send_header(
                "Access-Control-Allow-Headers", "content-type, x-frame-checksum"
            )
            self.send_header("Access-Control-Max-Age", "60")
            self.send_header("Cache-Control", "no-store, max-age=0")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self._error(HTTPStatus.NOT_FOUND, "not_found", "Resource unavailable.")

    def do_HEAD(self) -> None:  # noqa: N802 - stdlib handler contract
        self._dispatch(head_only=True)

    def do_GET(self) -> None:  # noqa: N802 - stdlib handler contract
        self._dispatch(head_only=False)

    def do_POST(self) -> None:  # noqa: N802 - stdlib handler contract
        path = urlsplit(self.path).path
        if path == "/api/v1/uploads/intents":
            self._upload_intent()
            return
        if path.startswith("/api/v1/uploads/") and path.endswith("/finalize"):
            self._finalize(path)
            return
        if path.startswith("/api/v1/media-jobs/") and path.endswith("/process"):
            self._process(path)
            return
        self._error(HTTPStatus.NOT_FOUND, "not_found", "Resource unavailable.")

    def do_PATCH(self) -> None:  # noqa: N802 - stdlib handler contract
        path = urlsplit(self.path).path
        if path.startswith("/api/v1/videos/") and path.endswith("/privacy"):
            self._privacy(path)
            return
        self._error(HTTPStatus.NOT_FOUND, "not_found", "Resource unavailable.")

    def do_PUT(self) -> None:  # noqa: N802 - stdlib handler contract
        path = urlsplit(self.path).path
        if path.startswith("/fake-r2/uploads/"):
            self._put_object(path)
            return
        self._error(HTTPStatus.NOT_FOUND, "not_found", "Resource unavailable.")

    def _dispatch(self, head_only: bool) -> None:
        path = urlsplit(self.path).path
        if path == "/__hermetic/health":
            self._json(
                HTTPStatus.OK,
                {
                    "status": "ready",
                    "component": "provider-semantic-fake",
                    "simulated": True,
                },
                head_only=head_only,
            )
            return
        if path == ASSET_PATH:
            self._asset(head_only)
            return
        if path.startswith("/api/v1/public/shares/") and path.endswith("/media"):
            self._media(path, head_only)
            return
        if path.startswith("/api/v1/public/shares/"):
            self._share(path, head_only)
            return
        self._error(HTTPStatus.NOT_FOUND, "not_found", "Resource unavailable.")

    def _upload_intent(self) -> None:
        if not self._actor_allowed():
            return
        body = self._read_json()
        if body is None:
            return
        fixture = self.server.fixture
        required = {
            "schema_version": 1,
            "tenant_id": fixture["tenant_id"],
            "video_id": fixture["video_id"],
            "expected_bytes": fixture["bytes"],
            "content_type": fixture["content_type"],
            "checksum_sha256": fixture["sha256"],
        }
        if body != required:
            self._error(
                HTTPStatus.UNPROCESSABLE_ENTITY,
                "invalid_upload_intent",
                "Synthetic upload intent is invalid.",
            )
            return
        upload_id = fixture["upload_id"]
        with self._db() as db:
            existing = db.execute(
                "SELECT tenant_id, video_id, expected_bytes, checksum_sha256, "
                "content_type FROM uploads WHERE id = ?",
                (upload_id,),
            ).fetchone()
            expected = (
                fixture["tenant_id"],
                fixture["video_id"],
                fixture["bytes"],
                fixture["sha256"],
                fixture["content_type"],
            )
            if existing is not None and existing != expected:
                self._error(
                    HTTPStatus.CONFLICT,
                    "intent_conflict",
                    "Synthetic upload intent conflicts with existing state.",
                )
                return
            if existing is None:
                db.execute(
                    """
                    INSERT INTO uploads (
                      id, tenant_id, video_id, expected_bytes, checksum_sha256,
                      content_type, state, etag, source_path
                    ) VALUES (?, ?, ?, ?, ?, ?, 'intent', NULL, NULL)
                    """,
                    (upload_id, *expected),
                )
        host = self.headers.get("Host", "127.0.0.1")
        self._json(
            HTTPStatus.CREATED,
            {
                "schema_version": 1,
                "upload_id": upload_id,
                "method": "PUT",
                "upload_url": f"http://{host}/fake-r2/uploads/{upload_id}",
                "required_headers": {
                    "content-type": fixture["content_type"],
                    "x-frame-checksum": fixture["sha256"],
                },
                "provider": "hermetic-r2-semantic-fake",
                "simulated": True,
            },
        )

    def _put_object(self, path: str) -> None:
        if self.headers.get("Origin") != self.server.web_origin:
            self._error(
                HTTPStatus.FORBIDDEN,
                "origin_forbidden",
                "Upload origin is not permitted.",
            )
            return
        upload_id = path.removeprefix("/fake-r2/uploads/")
        with self._db() as db:
            row = db.execute(
                "SELECT expected_bytes, checksum_sha256, content_type, state "
                "FROM uploads WHERE id = ?",
                (upload_id,),
            ).fetchone()
        if row is None or row[3] not in ("intent", "uploaded"):
            self._error(HTTPStatus.NOT_FOUND, "not_found", "Upload unavailable.")
            return
        if self.headers.get("Content-Type") != row[2]:
            self._error(
                HTTPStatus.UNSUPPORTED_MEDIA_TYPE,
                "content_type_mismatch",
                "Upload content type does not match the intent.",
            )
            return
        body = self._read_body()
        if body is None:
            return
        digest = hashlib.sha256(body).hexdigest()
        if len(body) != row[0] or digest != row[1]:
            self._error(
                HTTPStatus.UNPROCESSABLE_ENTITY,
                "object_mismatch",
                "Upload bytes do not match the intent.",
            )
            return
        if self.headers.get("X-Frame-Checksum") != digest:
            self._error(
                HTTPStatus.BAD_REQUEST,
                "checksum_header_mismatch",
                "Upload checksum header is invalid.",
            )
            return
        destination = self.server.state_dir / "objects" / f"{upload_id}.source"
        destination.write_bytes(body)
        etag = f'"sha256-{digest[:24]}"'
        with self._db() as db:
            db.execute(
                "UPDATE uploads SET state = 'uploaded', etag = ?, source_path = ? "
                "WHERE id = ?",
                (etag, str(destination), upload_id),
            )
        self.send_response(HTTPStatus.CREATED)
        self._cors_headers()
        self._dynamic_headers()
        self.send_header("ETag", etag)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def _finalize(self, path: str) -> None:
        if not self._actor_allowed():
            return
        upload_id = path.removeprefix("/api/v1/uploads/").removesuffix("/finalize")
        body = self._read_json()
        if body is None:
            return
        if set(body) != {"etag"}:
            self._error(
                HTTPStatus.UNPROCESSABLE_ENTITY,
                "invalid_finalize",
                "Synthetic finalization proof is invalid.",
            )
            return
        fixture = self.server.fixture
        with self._db() as db:
            row = db.execute(
                "SELECT state, etag, source_path FROM uploads WHERE id = ?",
                (upload_id,),
            ).fetchone()
            if row is None or row[0] not in ("uploaded", "finalized"):
                self._error(
                    HTTPStatus.CONFLICT,
                    "upload_not_complete",
                    "Upload cannot be finalized.",
                )
                return
            if body.get("etag") != row[1]:
                self._error(
                    HTTPStatus.UNPROCESSABLE_ENTITY,
                    "etag_mismatch",
                    "Finalization proof does not match the upload.",
                )
                return
            db.execute("UPDATE uploads SET state = 'finalized' WHERE id = ?", (upload_id,))
            db.execute(
                """
                INSERT OR REPLACE INTO videos (
                  id, tenant_id, title, privacy, state, playback_path,
                  bytes, content_type, checksum_sha256, revision
                ) VALUES (?, ?, 'Hermetic walking slice', 'public', 'processing',
                          NULL, ?, ?, ?, 1)
                """,
                (
                    fixture["video_id"],
                    fixture["tenant_id"],
                    fixture["bytes"],
                    fixture["content_type"],
                    fixture["sha256"],
                ),
            )
            db.execute(
                """
                INSERT OR REPLACE INTO jobs (
                  id, video_id, state, attempt, selected_executor, error_code
                ) VALUES (?, ?, 'queued', 0, NULL, NULL)
                """,
                (fixture["job_id"], fixture["video_id"]),
            )
            db.execute(
                """
                INSERT OR REPLACE INTO objects (
                  object_key, video_id, role, state, bytes, checksum_sha256, path
                ) VALUES (?, ?, 'source', 'available', ?, ?, ?)
                """,
                (
                    f"synthetic-source/{fixture['video_id']}/v1",
                    fixture["video_id"],
                    fixture["bytes"],
                    fixture["sha256"],
                    row[2],
                ),
            )
        self._json(
            HTTPStatus.ACCEPTED,
            {
                "schema_version": 1,
                "video_id": fixture["video_id"],
                "job_id": fixture["job_id"],
                "state": "processing",
                "simulated": True,
            },
        )

    def _process(self, path: str) -> None:
        if not self._actor_allowed():
            return
        job_id = path.removeprefix("/api/v1/media-jobs/").removesuffix("/process")
        body = self._read_json()
        if body is None:
            return
        if set(body) != {"executor"}:
            self._error(
                HTTPStatus.UNPROCESSABLE_ENTITY,
                "invalid_executor",
                "Synthetic executor selection is invalid.",
            )
            return
        executor = body.get("executor")
        fixture = self.server.fixture
        with self._db() as db:
            job = db.execute(
                "SELECT video_id, state, attempt FROM jobs WHERE id = ?", (job_id,)
            ).fetchone()
            upload = db.execute(
                "SELECT source_path FROM uploads WHERE id = ? AND state = 'finalized'",
                (fixture["upload_id"],),
            ).fetchone()
            if job is None or upload is None:
                self._error(HTTPStatus.NOT_FOUND, "not_found", "Job unavailable.")
                return
            if executor == "media_fake":
                db.execute(
                    "UPDATE jobs SET attempt = attempt + 1, selected_executor = ?, "
                    "error_code = 'injected_unavailable' WHERE id = ?",
                    (executor, job_id),
                )
                self._error(
                    HTTPStatus.SERVICE_UNAVAILABLE,
                    "media_fake_unavailable",
                    "Synthetic managed executor is unavailable.",
                    retry="later",
                )
                return
            if executor != "native_fallback":
                self._error(
                    HTTPStatus.UNPROCESSABLE_ENTITY,
                    "invalid_executor",
                    "Synthetic executor selection is invalid.",
                )
                return
            derivative = self.server.state_dir / "objects" / f"{job_id}.playback"
            shutil.copyfile(upload[0], derivative)
            digest = hashlib.sha256(derivative.read_bytes()).hexdigest()
            if digest != fixture["sha256"]:
                self._error(
                    HTTPStatus.INTERNAL_SERVER_ERROR,
                    "derivative_mismatch",
                    "Synthetic derivative validation failed.",
                )
                return
            db.execute(
                "UPDATE jobs SET state = 'succeeded', attempt = attempt + 1, "
                "selected_executor = ?, error_code = NULL WHERE id = ?",
                (executor, job_id),
            )
            db.execute(
                "UPDATE videos SET state = 'ready', playback_path = ?, revision = revision + 1 "
                "WHERE id = ?",
                (str(derivative), fixture["video_id"]),
            )
            db.execute(
                """
                INSERT OR REPLACE INTO objects (
                  object_key, video_id, role, state, bytes, checksum_sha256, path
                ) VALUES (?, ?, 'playback', 'available', ?, ?, ?)
                """,
                (
                    fixture["object_key"],
                    fixture["video_id"],
                    fixture["bytes"],
                    fixture["sha256"],
                    str(derivative),
                ),
            )
        self._json(
            HTTPStatus.OK,
            {
                "schema_version": 1,
                "job_id": job_id,
                "state": "succeeded",
                "executor": "native_fallback",
                "simulated": True,
            },
        )

    def _share(self, path: str, head_only: bool) -> None:
        video_id = path.removeprefix("/api/v1/public/shares/")
        with self._db() as db:
            row = db.execute(
                "SELECT title, privacy, state, bytes, content_type FROM videos WHERE id = ?",
                (video_id,),
            ).fetchone()
        if row is None or row[1] not in ("public", "unlisted"):
            self._json(HTTPStatus.OK, unavailable_share(), head_only=head_only)
            return
        if row[2] == "processing":
            body = unavailable_share()
            body["availability"] = "processing"
            body["canonical_url"] = f"{self.server.web_origin}/s/{video_id}"
            self._json(HTTPStatus.OK, body, head_only=head_only)
            return
        if row[2] != "ready":
            self._json(HTTPStatus.OK, unavailable_share(), head_only=head_only)
            return
        self._json(
            HTTPStatus.OK,
            {
                "api_version": {"major": 1},
                "availability": "public",
                "title": row[0],
                "description": None,
                "canonical_url": f"{self.server.web_origin}/s/{video_id}",
                "duration_ms": 2000,
                "playback": {
                    "path": f"/api/v1/public/shares/{video_id}/media",
                    "content_type": row[4],
                    "supports_range": True,
                    "captions": [],
                },
            },
            head_only=head_only,
        )

    def _media(self, path: str, head_only: bool) -> None:
        video_id = path.removeprefix("/api/v1/public/shares/").removesuffix("/media")
        cached = self.server.media_cache.get(path)
        with self._db() as db:
            row = db.execute(
                "SELECT privacy, state, playback_path, bytes, content_type, checksum_sha256 "
                "FROM videos WHERE id = ?",
                (video_id,),
            ).fetchone()
        if row is None or row[0] not in ("public", "unlisted") or row[1] != "ready":
            if self.server.inject_private_cache_hit and cached is not None:
                self._bytes_response(
                    cached,
                    "video/webm",
                    head_only,
                    cache_result="HIT",
                    allow_range=False,
                )
                return
            self._error(HTTPStatus.NOT_FOUND, "not_found", "Resource unavailable.")
            return
        source = Path(row[2])
        if not source.is_file():
            self._error(
                HTTPStatus.SERVICE_UNAVAILABLE,
                "media_unavailable",
                "Media is temporarily unavailable.",
                retry="later",
            )
            return
        data = source.read_bytes()
        if len(data) != row[3] or hashlib.sha256(data).hexdigest() != row[5]:
            self._error(
                HTTPStatus.SERVICE_UNAVAILABLE,
                "media_unavailable",
                "Media is temporarily unavailable.",
                retry="later",
            )
            return
        self.server.media_cache[path] = data
        self._bytes_response(
            data,
            row[4],
            head_only,
            cache_result="BYPASS",
            allow_range=True,
        )

    def _privacy(self, path: str) -> None:
        if not self._actor_allowed():
            return
        body = self._read_json()
        if body is None:
            return
        if body != {"privacy": "private"}:
            self._error(
                HTTPStatus.UNPROCESSABLE_ENTITY,
                "invalid_privacy",
                "Synthetic privacy transition is invalid.",
            )
            return
        video_id = path.removeprefix("/api/v1/videos/").removesuffix("/privacy")
        with self._db() as db:
            changed = db.execute(
                "UPDATE videos SET privacy = 'private', revision = revision + 1 WHERE id = ?",
                (video_id,),
            ).rowcount
        if changed != 1:
            self._error(HTTPStatus.NOT_FOUND, "not_found", "Resource unavailable.")
            return
        media_path = f"/api/v1/public/shares/{video_id}/media"
        purged = False
        if not self.server.inject_private_cache_hit:
            purged = self.server.media_cache.pop(media_path, None) is not None
        self._json(
            HTTPStatus.OK,
            {
                "schema_version": 1,
                "privacy": "private",
                "cache_purged": purged,
                "simulated": True,
            },
        )

    def _asset(self, head_only: bool) -> None:
        result = "HIT" if self.server.asset_seen else "MISS"
        self.server.asset_seen = True
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "application/javascript; charset=utf-8")
        self.send_header("Content-Length", str(len(ASSET_BODY)))
        self.send_header("Cache-Control", "public, max-age=31536000, immutable")
        self.send_header("X-Hermetic-Cache", result)
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        if not head_only:
            self.wfile.write(ASSET_BODY)

    def _bytes_response(
        self,
        data: bytes,
        content_type: str,
        head_only: bool,
        cache_result: str,
        allow_range: bool,
    ) -> None:
        status = HTTPStatus.OK
        body = data
        content_range: str | None = None
        if allow_range and self.headers.get("Range"):
            parsed = parse_range(self.headers["Range"], len(data))
            if parsed is None:
                self.send_response(HTTPStatus.REQUESTED_RANGE_NOT_SATISFIABLE)
                self._dynamic_headers()
                self.send_header("Content-Range", f"bytes */{len(data)}")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            start, end = parsed
            status = HTTPStatus.PARTIAL_CONTENT
            body = data[start : end + 1]
            content_range = f"bytes {start}-{end}/{len(data)}"
        self.send_response(status)
        self._dynamic_headers()
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", f'"sha256-{hashlib.sha256(data).hexdigest()[:24]}"')
        self.send_header("X-Hermetic-Cache", cache_result)
        if content_range:
            self.send_header("Content-Range", content_range)
        self.end_headers()
        if not head_only:
            self.wfile.write(body)

    def _actor_allowed(self) -> bool:
        if self.headers.get("X-Hermetic-Actor") == "synthetic-owner":
            return True
        self._error(
            HTTPStatus.UNAUTHORIZED,
            "unauthenticated",
            "Synthetic actor proof is required.",
        )
        return False

    def _read_json(self) -> dict[str, Any] | None:
        if self.headers.get("Content-Type") != "application/json":
            self._error(
                HTTPStatus.UNSUPPORTED_MEDIA_TYPE,
                "unsupported_content_type",
                "JSON is required.",
            )
            return None
        body = self._read_body()
        if body is None:
            return None
        try:
            value = json.loads(body)
        except (UnicodeDecodeError, json.JSONDecodeError):
            self._error(HTTPStatus.BAD_REQUEST, "invalid_json", "JSON is invalid.")
            return None
        if not isinstance(value, dict):
            self._error(HTTPStatus.BAD_REQUEST, "invalid_json", "JSON is invalid.")
            return None
        return value

    def _read_body(self) -> bytes | None:
        raw_length = self.headers.get("Content-Length")
        if raw_length is None or not raw_length.isdigit():
            self._error(
                HTTPStatus.LENGTH_REQUIRED,
                "content_length_required",
                "A bounded content length is required.",
            )
            return None
        length = int(raw_length)
        if length <= 0 or length > MAX_BODY:
            self._error(
                HTTPStatus.REQUEST_ENTITY_TOO_LARGE,
                "payload_too_large",
                "Request body is outside the synthetic limit.",
            )
            return None
        body = self.rfile.read(length)
        if len(body) != length:
            self._error(HTTPStatus.BAD_REQUEST, "truncated_body", "Request body is invalid.")
            return None
        return body

    def _db(self) -> sqlite3.Connection:
        connection = sqlite3.connect(self.server.db_path, timeout=2.0)
        connection.execute("PRAGMA foreign_keys = ON")
        return connection

    def _cors_headers(self) -> None:
        if self.headers.get("Origin") == self.server.web_origin:
            self.send_header("Access-Control-Allow-Origin", self.server.web_origin)
            self.send_header("Vary", "Origin")

    def _dynamic_headers(self) -> None:
        self.send_header("Cache-Control", "no-store, max-age=0")
        self.send_header("Pragma", "no-cache")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("X-Hermetic-Component", "provider-semantic-fake")

    def _json(
        self, status: HTTPStatus, value: dict[str, Any], head_only: bool = False
    ) -> None:
        body = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
        self.send_response(status)
        self._dynamic_headers()
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if not head_only:
            self.wfile.write(body)

    def _error(
        self,
        status: HTTPStatus,
        code: str,
        message: str,
        retry: str = "never",
    ) -> None:
        self._json(
            status,
            {
                "code": code,
                "message": message,
                "request_id": "hermetic-redacted",
                "retry": retry,
            },
        )


def unavailable_share() -> dict[str, Any]:
    return {
        "api_version": {"major": 1},
        "availability": "unavailable",
        "title": None,
        "description": None,
        "canonical_url": None,
        "duration_ms": None,
        "playback": None,
    }


def parse_range(value: str, size: int) -> tuple[int, int] | None:
    match = RANGE_RE.fullmatch(value)
    if not match or size <= 0:
        return None
    start_text, end_text = match.groups()
    if not start_text and not end_text:
        return None
    if not start_text:
        suffix = int(end_text)
        if suffix <= 0:
            return None
        length = min(suffix, size)
        return size - length, size - 1
    start = int(start_text)
    if start >= size:
        return None
    end = size - 1 if not end_text else min(int(end_text), size - 1)
    if end < start:
        return None
    return start, end


def initialize_database(path: Path) -> None:
    with sqlite3.connect(path) as db:
        db.executescript(
            """
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            CREATE TABLE uploads (
              id TEXT PRIMARY KEY,
              tenant_id TEXT NOT NULL,
              video_id TEXT NOT NULL,
              expected_bytes INTEGER NOT NULL CHECK (expected_bytes > 0),
              checksum_sha256 TEXT NOT NULL CHECK (length(checksum_sha256) = 64),
              content_type TEXT NOT NULL,
              state TEXT NOT NULL CHECK (state IN ('intent', 'uploaded', 'finalized')),
              etag TEXT,
              source_path TEXT
            );
            CREATE TABLE videos (
              id TEXT PRIMARY KEY,
              tenant_id TEXT NOT NULL,
              title TEXT NOT NULL,
              privacy TEXT NOT NULL CHECK (privacy IN ('public', 'unlisted', 'private')),
              state TEXT NOT NULL CHECK (state IN ('processing', 'ready', 'failed')),
              playback_path TEXT,
              bytes INTEGER NOT NULL,
              content_type TEXT NOT NULL,
              checksum_sha256 TEXT NOT NULL,
              revision INTEGER NOT NULL CHECK (revision > 0)
            );
            CREATE TABLE jobs (
              id TEXT PRIMARY KEY,
              video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
              state TEXT NOT NULL CHECK (state IN ('queued', 'succeeded', 'failed')),
              attempt INTEGER NOT NULL CHECK (attempt >= 0),
              selected_executor TEXT,
              error_code TEXT
            );
            CREATE TABLE objects (
              object_key TEXT PRIMARY KEY,
              video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
              role TEXT NOT NULL CHECK (role IN ('source', 'playback')),
              state TEXT NOT NULL CHECK (state = 'available'),
              bytes INTEGER NOT NULL CHECK (bytes > 0),
              checksum_sha256 TEXT NOT NULL CHECK (length(checksum_sha256) = 64),
              path TEXT NOT NULL
            );
            """
        )


def load_fixture(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    required = {
        "tenant_id",
        "video_id",
        "upload_id",
        "job_id",
        "object_key",
        "content_type",
        "bytes",
        "sha256",
    }
    if not isinstance(value, dict) or not required.issubset(value):
        raise ValueError("invalid hermetic fixture manifest")
    return value


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--ready-file", type=Path)
    parser.add_argument("--state-dir", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--web-origin", required=True)
    parser.add_argument("--inject-private-cache-hit", action="store_true")
    args = parser.parse_args()
    if args.port != 0 and not (1024 <= args.port <= 65535):
        parser.error("port must be zero or non-privileged")
    fixture = load_fixture(args.fixture)
    args.state_dir.mkdir(parents=True, exist_ok=True)
    server = HermeticServer(
        ("127.0.0.1", args.port),
        args.state_dir,
        fixture,
        args.web_origin,
        args.inject_private_cache_hit,
    )
    if args.ready_file is not None:
        # The journey treats existence as publication. Write the complete port
        # to a sibling first so readers can never observe an empty/partial file.
        temporary_ready_file = args.ready_file.with_name(f".{args.ready_file.name}.tmp")
        temporary_ready_file.write_text(f"{server.server_port}\n", encoding="ascii")
        os.replace(temporary_ready_file, args.ready_file)
    try:
        server.serve_forever(poll_interval=0.1)
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
