#!/usr/bin/env python3
"""Exercise Issue 31 SSR auth/dashboard routes against a loopback Frame web server."""

from __future__ import annotations

import argparse
import json
import math
import pathlib
import statistics
import time
import urllib.error
import urllib.parse
import urllib.request


ROOT = pathlib.Path(__file__).resolve().parents[2]
MATRIX = json.loads(
    (ROOT / "fixtures/web-authenticated/v1/route-matrix.json").read_text(encoding="utf-8")
)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"web authenticated parity: {message}")


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # type: ignore[no-untyped-def]
        return None


OPENER = urllib.request.build_opener(NoRedirect)


def request(
    origin: str,
    path: str,
    *,
    fields: dict[str, str] | None = None,
) -> tuple[int, dict[str, str], bytes, float]:
    data = None
    headers: dict[str, str] = {}
    method = "GET"
    if fields is not None:
        data = urllib.parse.urlencode(fields).encode("ascii")
        headers["Content-Type"] = "application/x-www-form-urlencoded"
        method = "POST"
    started = time.perf_counter()
    target = urllib.request.Request(
        f"{origin}{path}", data=data, headers=headers, method=method
    )
    try:
        response = OPENER.open(target, timeout=5)
    except urllib.error.HTTPError as error:
        response = error
    body = response.read()
    elapsed_ms = (time.perf_counter() - started) * 1_000
    return (
        int(response.status),
        {key.lower(): value for key, value in response.headers.items()},
        body,
        elapsed_ms,
    )


def validate_private_document(
    path: str,
    status: int,
    headers: dict[str, str],
    body: bytes,
    expected_status: int,
) -> str:
    require(status == expected_status, f"{path} returned {status}, expected {expected_status}")
    require(len(body) <= MATRIX["budgets"]["html_bytes"], f"{path} exceeds HTML budget")
    text = body.decode("utf-8")
    require(text.startswith("<!doctype html>"), f"{path} is not SSR HTML")
    require('<html lang="en">' in text, f"{path} has no document language")
    require('href="#main">Skip to content</a>' in text, f"{path} has no skip link")
    require('id="main"' in text and 'tabindex="-1"' in text, f"{path} has no focus target")
    require("<h1" in text and 'id="page-title"' in text, f"{path} has no page heading")
    require('name="robots" content="noindex,nofollow"' in text, f"{path} metadata is indexable")
    require(headers.get("cache-control") == "no-store", f"{path} is cacheable")
    require(headers.get("x-robots-tag") == "noindex,nofollow", f"{path} lacks robot header")
    require(headers.get("x-frame-options") == "DENY", f"{path} permits framing")
    require("frame-ancestors 'none'" in headers.get("content-security-policy", ""), f"{path} CSP permits framing")
    require("display-capture=()" in headers.get("permissions-policy", ""), f"{path} capture policy drifted")
    for forbidden in (
        "private-person@example.test",
        "fixture-secret",
        "signed=",
        "object_key",
        "provider_token",
        "session_cookie",
    ):
        require(forbidden not in text, f"{path} leaked {forbidden}")
    return text


def percentile_95(values: list[float]) -> float:
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * 0.95) - 1)]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--origin", default="http://127.0.0.1:3810")
    parser.add_argument("--evidence", type=pathlib.Path)
    args = parser.parse_args()
    parsed = urllib.parse.urlsplit(args.origin.rstrip("/"))
    require(
        parsed.scheme == "http"
        and parsed.hostname in {"127.0.0.1", "::1"}
        and parsed.port is not None
        and parsed.username is None
        and parsed.password is None
        and parsed.path in {"", "/"}
        and not parsed.query
        and not parsed.fragment,
        "--origin must be an exact loopback HTTP origin with a port",
    )
    host = f"[{parsed.hostname}]" if parsed.hostname == "::1" else parsed.hostname
    origin = f"http://{host}:{parsed.port}"
    timings: list[float] = []
    role_cases = 0
    denied_cases = 0

    for route in MATRIX["routes"]:
        path = route["fixture_path"]
        status, headers, body, elapsed = request(origin, path)
        timings.append(elapsed)
        unauthenticated = validate_private_document(path, status, headers, body, 401)
        require("Sign in required" in unauthenticated, f"{path} has no auth boundary")
        require("Local Frame workspace" not in unauthenticated, f"{path} flashes private content")
        canonical = f'{origin}{path}'
        require(f'rel="canonical" href="{canonical}"' in unauthenticated, f"{path} canonical drifted")

        for role in MATRIX["roles"]:
            expected = 200 if role in route["allowed_roles"] else 403
            status, headers, body, elapsed = request(origin, f"{path}?fixture={role}")
            timings.append(elapsed)
            text = validate_private_document(path, status, headers, body, expected)
            role_cases += 1
            if expected == 200:
                require("Local Frame workspace" in text, f"{path} {role} did not render ready state")
                require(f">{role.capitalize()}<" in text, f"{path} {role} role badge is absent")
            else:
                denied_cases += 1
                require("Access denied" in text, f"{path} {role} has no denial state")
                require("Local Frame workspace" not in text, f"{path} {role} denial leaked workspace")

    state_expectations = {
        "loading": (202, "Loading workspace"),
        "denied": (403, "Access denied"),
        "failed": (503, "Workspace unavailable"),
        "empty": (200, "Local empty workspace"),
    }
    for fixture, (expected_status, marker) in state_expectations.items():
        status, headers, body, elapsed = request(origin, f"/dashboard?fixture={fixture}")
        timings.append(elapsed)
        text = validate_private_document("/dashboard", status, headers, body, expected_status)
        require(marker in text, f"dashboard {fixture} state is absent")
        if fixture != "empty":
            require("Product walkthrough" not in text, f"dashboard {fixture} state leaked records")

    status, headers, body, elapsed = request(
        origin, "/library?fixture=owner&q=Product&filter=ready&page=1&theme=light"
    )
    timings.append(elapsed)
    filtered = validate_private_document("/library", status, headers, body, 200)
    require("Product walkthrough" in filtered, "library search dropped the matching recording")
    require("Weekly update" not in filtered, "library filter retained a nonmatching recording")
    require('data-theme="light"' in filtered, "explicit light theme was not preserved")
    require('value="Product"' in filtered, "normalized search was not preserved")
    for query in (
        "filter=unknown",
        "page=0",
        "theme=unknown",
        f"q={'x' * 121}",
    ):
        status, _, body, elapsed = request(origin, f"/library?fixture=owner&{query}")
        timings.append(elapsed)
        require(status == 400 and body == b"invalid view query", f"invalid query was accepted: {query}")

    for invalid_path in ("/spaces/not%2Fsafe", "/folders/%2e%2e"):
        status, _, _, elapsed = request(origin, f"{invalid_path}?fixture=owner")
        timings.append(elapsed)
        require(status in {400, 404}, f"unsafe resource path was accepted: {invalid_path}")

    alias_cases = 0
    for route in MATRIX["routes"]:
        for alias in route["legacy_aliases"]:
            status, headers, _, elapsed = request(origin, alias)
            timings.append(elapsed)
            require(status == 308, f"legacy alias did not permanently redirect: {alias}")
            require(headers.get("location") == route["fixture_path"], f"legacy alias target drifted: {alias}")
            alias_cases += 1

    auth_error_cases = 0
    render_auth_post_rejections = 0
    for auth in MATRIX["auth_routes"]:
        path = auth["path"]
        endpoint = auth["post_endpoint"]
        status, headers, body, elapsed = request(origin, path)
        timings.append(elapsed)
        text = validate_private_document(path, status, headers, body, 200)
        require(f'action="{endpoint}"' in text, f"{path} Worker form action drifted")
        require('method="post"' in text, f"{path} form is not POST")

        for error_state in ("invalid", "failed"):
            error_path = f"{path}?auth_error={error_state}"
            status, headers, body, elapsed = request(origin, error_path)
            timings.append(elapsed)
            text = validate_private_document(error_path, status, headers, body, 200)
            require('role="alert"' in text, f"{error_path} has no accessible failure state")
            require("auth_error=" not in text, f"{error_path} reflected its control query")
            auth_error_cases += 1

        private_probe = {"probe": "private-person@example.test"}
        for render_path, expected_status in ((path, 405), (endpoint, 404)):
            status, headers, body, elapsed = request(
                origin, render_path, fields=private_probe
            )
            timings.append(elapsed)
            require(
                status == expected_status,
                f"Render accepted auth POST {render_path}: {status}",
            )
            require(
                headers.get("location") is None,
                f"Render redirected auth POST {render_path}",
            )
            require(
                private_probe["probe"].encode("ascii") not in body,
                f"Render reflected auth material at {render_path}",
            )
            render_auth_post_rejections += 1

    p95_ms = percentile_95(timings)
    require(
        p95_ms <= MATRIX["budgets"]["server_render_p95_ms"],
        f"local SSR p95 {p95_ms:.2f} ms exceeds charter budget",
    )
    evidence = {
        "schema": "frame.web-authenticated-http-e2e.v1",
        "origin": origin,
        "route_count": len(MATRIX["routes"]),
        "role_route_cases": role_cases,
        "denied_cases": denied_cases,
        "state_cases": len(state_expectations),
        "auth_error_cases": auth_error_cases,
        "render_auth_post_rejections": render_auth_post_rejections,
        "legacy_alias_cases": alias_cases,
        "query_validation_cases": 4,
        "ssr_request_count": len(timings),
        "ssr_latency_p95_ms": round(p95_ms, 3),
        "ssr_latency_mean_ms": round(statistics.fmean(timings), 3),
        "ssr_budget_ms": MATRIX["budgets"]["server_render_p95_ms"],
        "private_html_no_store": True,
        "private_metadata_noindex": True,
        "flash_of_private_content": False,
        "server_role_filtering": True,
        "unsafe_query_rejected": True,
        "auth_adapter_status": "worker_direct_render_post_rejected",
    }
    rendered = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
