#!/usr/bin/env python3
"""Exercise Issue 39's public same-origin route without changing provider state."""

from __future__ import annotations

import argparse
import contextlib
import http.client
import json
import pathlib
import re
import ssl
import sys
import threading
from collections.abc import Iterable, Iterator
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import urlsplit


ROOT = pathlib.Path(__file__).resolve().parents[2]
MATRIX_PATH = ROOT / "fixtures" / "same-origin-routing" / "v1" / "route-owner-matrix.json"
ALLOWED_ORIGINS = {
    "https://frame.engmanager.xyz",
    "https://frame-staging.engmanager.xyz",
}
SELF_TEST_SHARE = "018f47a6-7b1c-7f55-8f39-8f8a8690f123"


class ConformanceError(RuntimeError):
    """A live same-origin assertion failed."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ConformanceError(message)


class Response:
    def __init__(self, status: int, headers: dict[str, str], body: bytes) -> None:
        self.status = status
        self.headers = headers
        self.body = body

    def json(self) -> dict[str, Any]:
        value = json.loads(self.body)
        require(isinstance(value, dict), "response JSON was not an object")
        return value


class Client:
    def __init__(self, origin: str) -> None:
        parsed = urlsplit(origin)
        require(parsed.scheme in {"http", "https"}, "origin scheme must be HTTP(S)")
        require(bool(parsed.hostname) and parsed.path == "" and parsed.query == "", "origin must contain only scheme and authority")
        self.scheme = parsed.scheme
        self.host = parsed.hostname or ""
        self.port = parsed.port

    def request(
        self,
        method: str,
        path: str,
        *,
        headers: dict[str, str] | None = None,
        body: bytes | Iterable[bytes] | None = None,
        encode_chunked: bool = False,
    ) -> Response:
        require(path.startswith("/") and "#" not in path, f"invalid test path {path!r}")
        if self.scheme == "https":
            connection: http.client.HTTPConnection = http.client.HTTPSConnection(
                self.host,
                self.port,
                timeout=20,
                context=ssl.create_default_context(),
            )
        else:
            connection = http.client.HTTPConnection(self.host, self.port, timeout=5)
        try:
            connection.request(method, path, body=body, headers=headers or {}, encode_chunked=encode_chunked)
            raw = connection.getresponse()
            payload = raw.read(65_537)
            require(len(payload) <= 65_536, f"{method} {path}: response exceeded evidence bound")
            response_headers: dict[str, str] = {}
            for name, value in raw.getheaders():
                lower = name.lower()
                response_headers[lower] = f"{response_headers[lower]}, {value}" if lower in response_headers else value
            return Response(raw.status, response_headers, payload)
        finally:
            connection.close()


def no_shared_cache(response: Response, label: str) -> None:
    cache_control = response.headers.get("cache-control", "").lower()
    require("no-store" in cache_control or "private" in cache_control, f"{label}: missing private cache policy")
    cache_status = response.headers.get("cf-cache-status", "DYNAMIC").upper()
    require(cache_status in {"DYNAMIC", "BYPASS"}, f"{label}: shared cache status was {cache_status}")


def worker_response(response: Response, label: str) -> None:
    request_id = response.headers.get("x-request-id", "")
    require(re.fullmatch(r"r-[A-Za-z0-9-]{8,64}", request_id) is not None, f"{label}: missing normalized Worker request ID")
    ray = response.headers.get("cf-ray")
    if ray is not None:
        require(request_id == f"r-{ray}", f"{label}: Worker and Cloudflare request IDs do not identify one hop")
    no_shared_cache(response, label)


def request_path(url: str) -> str:
    parsed = urlsplit(url)
    return parsed.path + (f"?{parsed.query}" if parsed.query else "")


def exercise(origin: str, public_share_id: str | None, require_full: bool) -> dict[str, Any]:
    matrix = json.loads(MATRIX_PATH.read_text(encoding="utf-8"))
    client = Client(origin)
    route_results: list[dict[str, Any]] = []

    ready = client.request("GET", "/health/ready")
    require(ready.status == 200, f"Render readiness returned {ready.status}")
    require(ready.json().get("service") == "frame-web", "Render readiness did not identify frame-web")

    expected_api_success = {
        "api-discovery",
        "api-discovery-query",
        "api-discovery-slash",
        "api-capabilities",
        "api-capabilities-slash",
        "api-health",
    }
    for case in matrix["cases"]:
        owner = case["edge_owner"]
        if owner == "render":
            continue
        response = client.request("GET", request_path(case["url"]))
        if owner == "worker_api":
            worker_response(response, case["id"])
            require(response.status == 200, f"{case['id']}: expected 200, got {response.status}")
            require(case["id"] in expected_api_success, f"{case['id']}: unreviewed API success")
        elif owner == "worker_reject":
            worker_response(response, case["id"])
            require(response.status in {400, 404}, f"{case['id']}: expected closed 400/404, got {response.status}")
        else:
            # URL normalization can cause this spelling to miss the literal
            # route or be rejected at the edge. It must never become API
            # discovery/capabilities success.
            require(response.status >= 400, f"{case['id']}: encoded lookalike entered an API handler")
            no_shared_cache(response, case["id"])
        route_results.append({"id": case["id"], "status": response.status, "owner": owner})

    method_statuses: dict[str, int] = {}
    for method in ("GET", "HEAD", "POST", "PUT", "DELETE", "OPTIONS"):
        response = client.request(
            method,
            "/api?method-matrix=1",
            headers={"content-type": "application/json", "content-length": "2"},
            body=b"{}" if method in {"POST", "PUT"} else None,
        )
        worker_response(response, f"method-{method}")
        expected = 200 if method == "GET" else 405
        require(response.status == expected, f"{method} /api returned {response.status}, expected {expected}")
        require(not 300 <= response.status < 400, f"{method} /api introduced a redirect")
        require(response.status != 101, f"{method} /api upgraded protocols")
        method_statuses[method] = response.status

    chunked = client.request(
        "POST",
        "/api?chunked=1",
        headers={"content-type": "application/octet-stream"},
        body=iter((b"route-", b"body")),
        encode_chunked=True,
    )
    worker_response(chunked, "chunked-body")
    require(chunked.status == 405, f"chunked request route returned {chunked.status}")

    upgrade = client.request(
        "GET",
        "/api?upgrade=1",
        headers={"connection": "upgrade", "upgrade": "websocket"},
    )
    require(upgrade.status != 101, "API route accepted an unsupported protocol upgrade")
    worker_response(upgrade, "upgrade")

    spoof = client.request(
        "GET",
        "/api?request-id=1",
        headers={
            "x-request-id": "attacker-controlled",
            "cf-ray": "attacker123-SJC",
            "forwarded": "host=evil.example;proto=http",
            "x-forwarded-host": "evil.example",
        },
    )
    worker_response(spoof, "spoofed-metadata")
    require(spoof.headers.get("x-request-id") != "attacker-controlled", "client selected the response request ID")

    poisoned_host = client.request("GET", "/api?host-poison=1", headers={"host": "evil.example"})
    require(poisoned_host.status >= 400, f"unexpected Host returned {poisoned_host.status}")
    require(not 300 <= poisoned_host.status < 400, "unexpected Host produced a canonical redirect")

    range_result: dict[str, Any] = {"status": "protected_fixture_not_supplied"}
    if public_share_id is not None:
        require(re.fullmatch(r"[0-9a-f]{8}-[0-9a-f-]{27}", public_share_id) is not None, "public share ID shape is invalid")
        path = f"/api/v1/public/shares/{public_share_id}/media"
        ranged = client.request("GET", path, headers={"range": "bytes=0-0"})
        worker_response(ranged, "single-range")
        require(ranged.status == 206 and ranged.headers.get("content-range", "").startswith("bytes 0-0/"), "single range did not preserve 206 semantics")
        require(ranged.headers.get("accept-ranges", "").lower() == "bytes", "single range omitted accept-ranges")
        multi = client.request("GET", path, headers={"range": "bytes=0-0,2-2"})
        worker_response(multi, "multi-range")
        require(multi.status == 416, f"multiple ranges returned {multi.status}, expected 416")
        range_result = {"status": "passed", "single": 206, "multiple": 416}
    elif require_full:
        raise ConformanceError("--require-full needs --public-share-id for range and streamed-body evidence")

    return {
        "schema": "frame.same-origin-live-conformance.v1",
        "origin": origin,
        "route_cases": route_results,
        "method_statuses": method_statuses,
        "chunked_body_status": chunked.status,
        "upgrade_status": upgrade.status,
        "request_id_spoof_rejected": True,
        "unexpected_host_status": poisoned_host.status,
        "range": range_result,
        "provider_state_changed": False,
    }


class FakeEdge(BaseHTTPRequestHandler):
    server_version = "frame-same-origin-self-test"
    sys_version = ""

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def _send(self, status: int, value: dict[str, Any] | bytes, *, worker: bool = False, extra: dict[str, str] | None = None) -> None:
        body = value if isinstance(value, bytes) else json.dumps(value, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("content-type", "application/octet-stream" if isinstance(value, bytes) else "application/json")
        self.send_header("content-length", str(len(body)))
        self.send_header("cache-control", "no-store, max-age=0")
        if worker:
            self.send_header("x-request-id", "r-selftest1234-SJC")
            self.send_header("cf-ray", "selftest1234-SJC")
            self.send_header("cf-cache-status", "DYNAMIC")
        for name, header_value in (extra or {}).items():
            self.send_header(name, header_value)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def _handle(self) -> None:
        path = self.path.split("?", 1)[0]
        if self.headers.get("host") == "evil.example":
            self._send(421, {"code": "unexpected_host"}, worker=True)
            return
        if path == "/health/ready":
            self._send(200, {"service": "frame-web", "status": "ready"})
            return
        if path == f"/api/v1/public/shares/{SELF_TEST_SHARE}/media":
            range_value = self.headers.get("range")
            if range_value == "bytes=0-0":
                self._send(206, b"x", worker=True, extra={"content-range": "bytes 0-0/1", "accept-ranges": "bytes"})
            elif range_value:
                self._send(416, {"code": "range_not_satisfiable"}, worker=True, extra={"content-range": "bytes */1"})
            else:
                self._send(200, b"x", worker=True, extra={"accept-ranges": "bytes"})
            return
        if path in {"/api", "/api/", "/api/v1", "/api/v1/", "/api/v1/health"}:
            if self.command == "GET":
                self._send(200, {"service": "frame", "api_version": {"major": 1}}, worker=True)
            else:
                self._send(405, {"code": "method_not_allowed"}, worker=True, extra={"allow": "GET"})
            return
        if path in {
            "/api//v1",
            "/api/./v1",
            "/api/../private",
            "/api/v1;admin",
            "/api/%2e%2e/private",
        }:
            self._send(400, {"code": "invalid_api_path"}, worker=True)
            return
        if path.startswith("/api"):
            self._send(404, {"code": "not_found"}, worker=True)
            return
        self._send(404, {"code": "render_not_found"})

    do_GET = _handle
    do_HEAD = _handle
    do_POST = _handle
    do_PUT = _handle
    do_DELETE = _handle
    do_OPTIONS = _handle


@contextlib.contextmanager
def fake_edge() -> Iterator[str]:
    server = ThreadingHTTPServer(("127.0.0.1", 0), FakeEdge)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        host, port = server.server_address
        yield f"http://{host}:{port}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--origin", choices=sorted(ALLOWED_ORIGINS))
    parser.add_argument("--public-share-id")
    parser.add_argument("--require-full", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--evidence", type=pathlib.Path)
    arguments = parser.parse_args()
    if arguments.self_test == (arguments.origin is not None):
        parser.error("select exactly one of --self-test or --origin")

    try:
        if arguments.self_test:
            with fake_edge() as origin:
                report = exercise(origin, SELF_TEST_SHARE, True)
                report["origin"] = "loopback_fake_edge"
                report["evidence_class"] = "runner_self_test"
        else:
            report = exercise(arguments.origin or "", arguments.public_share_id, arguments.require_full)
            report["evidence_class"] = "protected_public_trace"
    except (ConformanceError, OSError, http.client.HTTPException, json.JSONDecodeError) as error:
        print(f"same-origin live conformance failed: {error}", file=sys.stderr)
        return 1

    if arguments.evidence is not None:
        arguments.evidence.parent.mkdir(parents=True, exist_ok=True)
        arguments.evidence.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        "Same-origin live runner passed "
        f"({len(report['route_cases'])} routed cases; {report['evidence_class']}; provider state untouched)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
