#!/usr/bin/env python3
"""Credential-free Render composition-root, statelessness, and drain smoke."""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import pathlib
import signal
import socket
import subprocess
import tempfile
import threading
import time
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
SCHEMA = "frame.render-web-runtime-local.v1"
PRODUCTION = "https://frame.engmanager.xyz"
PREVIEW = "https://frame-pr-38.onrender.com"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def available_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def clean_environment(values: dict[str, str]) -> dict[str, str]:
    environment = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("FRAME_", "RENDER_"))
        and key not in {"PORT", "IS_PULL_REQUEST"}
    }
    environment.update(values)
    return environment


def request(
    port: int,
    host: str,
    path: str,
    *,
    headers: dict[str, str] | None = None,
    timeout: float = 2.0,
) -> tuple[int, dict[str, str], bytes]:
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    request_headers = {"Host": host, "Connection": "close", **(headers or {})}
    connection.request("GET", path, headers=request_headers)
    response = connection.getresponse()
    body = response.read(256 * 1024 + 1)
    response_headers = {key.lower(): value for key, value in response.getheaders()}
    status = response.status
    connection.close()
    require(len(body) <= 256 * 1024, f"{path} exceeded smoke response bound")
    return status, response_headers, body


class Server:
    def __init__(
        self,
        binary: pathlib.Path,
        environment: dict[str, str],
        workdir: pathlib.Path,
        log_path: pathlib.Path,
    ) -> None:
        self.log_path = log_path
        self.log_handle = log_path.open("wb")
        self.started_at = time.monotonic()
        self.process = subprocess.Popen(
            [str(binary)],
            cwd=workdir,
            env=clean_environment(environment),
            stdin=subprocess.DEVNULL,
            stdout=self.log_handle,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )

    def wait_http(self, port: int, host: str) -> float:
        deadline = time.monotonic() + 5.0
        last_error: Exception | None = None
        while time.monotonic() < deadline:
            require(self.process.poll() is None, f"server exited early: {self.log()}")
            try:
                status, _, _ = request(port, host, "/health/live", timeout=0.25)
                if status == 200:
                    return (time.monotonic() - self.started_at) * 1_000
            except (OSError, http.client.HTTPException) as error:
                last_error = error
            time.sleep(0.025)
        raise RuntimeError(f"server startup timed out ({last_error}): {self.log()}")

    def signal_and_wait(self, timeout: float = 3.0) -> tuple[float, int]:
        started = time.monotonic()
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
        try:
            return_code = self.process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait(timeout=2.0)
            raise RuntimeError(f"server did not drain before {timeout}s: {self.log()}")
        finally:
            self.log_handle.close()
        return (time.monotonic() - started) * 1_000, return_code

    def log(self) -> str:
        self.log_handle.flush()
        return self.log_path.read_text(encoding="utf-8", errors="replace")

    def kill(self) -> None:
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait(timeout=2.0)
        if not self.log_handle.closed:
            self.log_handle.close()


def production_environment(port: int) -> dict[str, str]:
    return {
        "PORT": str(port),
        "FRAME_DEPLOYMENT": "production",
        "FRAME_PUBLIC_ORIGIN": PRODUCTION,
        "FRAME_API_ORIGIN": PRODUCTION,
        "FRAME_PROXY_TRUST": "render",
        "FRAME_RELEASE_ID": "render-runtime-local",
        "RENDER_EXTERNAL_URL": "https://frame-web.onrender.com",
        "FRAME_ENABLE_PUBLIC_EMBED": "false",
        "RUST_LOG": "info",
    }


def preview_environment(port: int) -> dict[str, str]:
    return {
        "PORT": str(port),
        "FRAME_DEPLOYMENT": "preview",
        "IS_PULL_REQUEST": "true",
        "FRAME_PUBLIC_ORIGIN": "https://frame-preview.invalid",
        "FRAME_API_ORIGIN": "https://frame-staging.engmanager.xyz",
        "FRAME_PROXY_TRUST": "render",
        "FRAME_RELEASE_ID": "render-preview-local",
        "RENDER_EXTERNAL_URL": PREVIEW,
        "FRAME_ENABLE_PUBLIC_EMBED": "false",
        "RUST_LOG": "info",
    }


def run(binary: pathlib.Path) -> dict[str, Any]:
    require(binary.is_file(), f"missing frame-web binary: {binary}")
    require(os.access(binary, os.X_OK), f"frame-web is not executable: {binary}")
    require(
        (binary.parent / "web-dist" / "manifest.json").is_file(),
        "production smoke requires executable-adjacent web-dist/manifest.json",
    )
    binary_sha256 = hashlib.sha256(binary.read_bytes()).hexdigest()
    evidence: dict[str, Any] = {
        "schema": SCHEMA,
        "binary_sha256": binary_sha256,
        "cases": {},
        "protected_provider_evidence": False,
    }

    with tempfile.TemporaryDirectory(prefix="frame-render-runtime-") as raw_temp:
        temp = pathlib.Path(raw_temp)
        servers: list[Server] = []
        try:
            invalid_port = available_port()
            invalid_log = temp / "invalid.log"
            invalid_workdir = temp / "invalid-cwd"
            invalid_workdir.mkdir()
            invalid = Server(
                binary,
                {
                    **production_environment(invalid_port),
                    "FRAME_PUBLIC_ORIGIN": "https://invalid.example?token=DO_NOT_LOG",
                },
                invalid_workdir,
                invalid_log,
            )
            servers.append(invalid)
            invalid_code = invalid.process.wait(timeout=3.0)
            invalid.log_handle.close()
            invalid_text = invalid_log.read_text(encoding="utf-8", errors="replace")
            require(invalid_code != 0, "invalid production config unexpectedly started")
            require("DO_NOT_LOG" not in invalid_text, "config error leaked rejected input")
            evidence["cases"]["redacted_fail_fast"] = {"exit_code_nonzero": True}

            production_port = available_port()
            production_workdir = temp / "production-cwd"
            production_workdir.mkdir()
            production = Server(
                binary,
                production_environment(production_port),
                production_workdir,
                temp / "production.log",
            )
            servers.append(production)
            production_startup = production.wait_http(
                production_port, "frame.engmanager.xyz"
            )
            require(production_startup < 5_000, "production startup exceeded 5s")
            require(
                f"0.0.0.0:{production_port}" in production.log(),
                "PORT startup did not report the all-interface bind",
            )
            status, ready_headers, ready_body = request(
                production_port,
                "frame.engmanager.xyz",
                "/health/ready",
                headers={"X-Forwarded-Proto": "https"},
            )
            require(status == 200, f"production readiness returned {status}")
            ready = json.loads(ready_body)
            require(ready["status"] == "ready", "production was not ready")
            require(ready["configuration"] is True, "configuration readiness missing")
            require(ready["hydration_assets"] is True, "asset readiness missing")
            require(ready["public_ssr"] is True, "SSR client readiness missing")
            require(
                ready_headers.get("cache-control") == "no-store",
                "readiness must not be cached",
            )
            bad_proxy, _, _ = request(
                production_port,
                "frame.engmanager.xyz",
                "/health/live",
                headers={"X-Forwarded-Proto": "http"},
            )
            require(bad_proxy == 400, "Render proxy mode admitted an HTTP scheme marker")
            diagnostic, _, _ = request(
                production_port, "frame.engmanager.xyz", "/health/dependencies"
            )
            require(diagnostic == 404, "dependency diagnostics are not token-hidden")
            release_diagnostic, _, _ = request(
                production_port, "frame.engmanager.xyz", "/health/release"
            )
            require(
                release_diagnostic == 404,
                "release diagnostics are not token-hidden",
            )
            production_drain, production_code = production.signal_and_wait()
            require(production_code == 0, "production-mode SIGTERM exit was not clean")
            require(not list(production_workdir.iterdir()), "production wrote cwd state")
            evidence["cases"]["production"] = {
                "bind": f"0.0.0.0:{production_port}",
                "startup_ms": round(production_startup, 3),
                "sigterm_exit_ms": round(production_drain, 3),
                "ready": True,
                "proxy_policy": "render_https_only_when_present",
            }

            preview_port = available_port()
            preview_workdir = temp / "preview-cwd"
            preview_workdir.mkdir()
            preview = Server(
                binary,
                preview_environment(preview_port),
                preview_workdir,
                temp / "preview.log",
            )
            servers.append(preview)
            preview_startup = preview.wait_http(preview_port, "frame-pr-38.onrender.com")
            status, headers, body = request(
                preview_port, "frame-pr-38.onrender.com", "/"
            )
            text = body.decode("utf-8")
            require(status == 200, f"preview landing returned {status}")
            require(PREVIEW in text, "preview did not use RENDER_EXTERNAL_URL")
            require(PRODUCTION not in text, "preview emitted production canonical origin")
            require(
                "noindex" in headers.get("x-robots-tag", ""),
                "preview response was indexable",
            )
            require("set-cookie" not in headers, "preview issued a cookie")
            preview_drain, preview_code = preview.signal_and_wait()
            require(preview_code == 0, "preview SIGTERM exit was not clean")
            require(not list(preview_workdir.iterdir()), "preview wrote cwd state")
            evidence["cases"]["preview"] = {
                "public_origin_source": "RENDER_EXTERNAL_URL",
                "api_class": "non_production",
                "startup_ms": round(preview_startup, 3),
                "sigterm_exit_ms": round(preview_drain, 3),
                "noindex": True,
                "cookie_free": True,
            }

            local_responses: list[bytes] = []
            local_startups: list[float] = []
            for index in range(2):
                port = available_port()
                workdir = temp / f"local-{index}-cwd"
                workdir.mkdir()
                local = Server(
                    binary,
                    {
                        "FRAME_ADDR": f"127.0.0.1:{port}",
                        "FRAME_DEPLOYMENT": "local",
                        "FRAME_RELEASE_ID": "render-scale-local",
                        "RUST_LOG": "info",
                    },
                    workdir,
                    temp / f"local-{index}.log",
                )
                servers.append(local)
                local_startups.append(local.wait_http(port, f"127.0.0.1:{port}"))
                local_status, _, local_body = request(
                    port, f"127.0.0.1:{port}", "/health/live"
                )
                require(local_status == 200, "local scale instance was not live")
                proxy_status, _, _ = request(
                    port,
                    f"127.0.0.1:{port}",
                    "/health/live",
                    headers={"X-Forwarded-Proto": "https"},
                )
                require(proxy_status == 400, "direct mode trusted forwarding metadata")
                local_responses.append(local_body)
                _, code = local.signal_and_wait()
                require(code == 0, "local scale instance did not stop cleanly")
                require(not list(workdir.iterdir()), "local instance wrote cwd state")
            require(
                local_responses[0] == local_responses[1],
                "identical requests differed across stateless instances",
            )
            evidence["cases"]["scale_restart"] = {
                "instances": 2,
                "identical_response": True,
                "empty_workdirs": True,
                "startup_ms": [round(value, 3) for value in local_startups],
            }

            drain_port = available_port()
            drain_workdir = temp / "drain-cwd"
            drain_workdir.mkdir()
            drain = Server(
                binary,
                {
                    "FRAME_ADDR": f"127.0.0.1:{drain_port}",
                    "FRAME_DEPLOYMENT": "local",
                    "FRAME_RELEASE_ID": "render-drain-local",
                    "FRAME_RUNTIME_TEST_MODE": "true",
                    "RUST_LOG": "info",
                },
                drain_workdir,
                temp / "drain.log",
            )
            servers.append(drain)
            drain.wait_http(drain_port, f"127.0.0.1:{drain_port}")
            result: dict[str, Any] = {}
            request_started = threading.Event()

            def inflight() -> None:
                request_started.set()
                try:
                    result["response"] = request(
                        drain_port,
                        f"127.0.0.1:{drain_port}",
                        "/_internal/runtime/drain?delay_ms=400",
                        timeout=2.0,
                    )
                except Exception as error:  # retained for the main assertion
                    result["error"] = repr(error)

            thread = threading.Thread(target=inflight, daemon=True)
            thread.start()
            require(request_started.wait(1.0), "in-flight drain request did not start")
            time.sleep(0.15)
            signal_started = time.monotonic()
            drain.process.send_signal(signal.SIGTERM)
            thread.join(timeout=2.0)
            require(not thread.is_alive(), "in-flight request exceeded drain budget")
            require("error" not in result, f"in-flight request failed: {result.get('error')}")
            require(result["response"][0] == 204, "in-flight request did not complete")
            drain_code = drain.process.wait(timeout=2.0)
            drain.log_handle.close()
            drain_elapsed = (time.monotonic() - signal_started) * 1_000
            require(drain_code == 0, "drain process did not exit cleanly")
            require(drain_elapsed < 2_000, "local drain exceeded 2s smoke budget")
            evidence["cases"]["inflight_sigterm"] = {
                "request_completed": True,
                "exit_code": drain_code,
                "elapsed_ms": round(drain_elapsed, 3),
                "configured_render_budget_seconds": 60,
                "application_budget_seconds": 55,
            }
        finally:
            for server in servers:
                server.kill()

    return evidence


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--binary",
        type=pathlib.Path,
        default=ROOT / "target" / "release" / "frame-web",
    )
    parser.add_argument("--evidence", type=pathlib.Path)
    args = parser.parse_args()
    evidence = run(args.binary.resolve())
    serialized = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(serialized, encoding="utf-8")
    print(serialized, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
