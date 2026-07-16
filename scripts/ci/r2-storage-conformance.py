#!/usr/bin/env python3
"""Exercise ObjectStoreV1 through Wrangler's credential-free local R2 binding."""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import pathlib
import re
import socket
import subprocess
import sys
import tempfile
import time
from collections.abc import Sequence
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
RUNNER_SOURCE = pathlib.Path(__file__).resolve()
CONFIG = ROOT / "apps" / "control-plane" / "wrangler.local.toml"
R2_SOURCE = ROOT / "apps" / "control-plane" / "src" / "r2_storage.rs"
ROUTING_SOURCE = ROOT / "apps" / "control-plane" / "src" / "routing.rs"
LIB_SOURCE = ROOT / "apps" / "control-plane" / "src" / "lib.rs"
CONFORMANCE_PATH = "/__frame/local/r2-storage-conformance"
WRANGLER_VERSION = "4.111.0"
ANSI = re.compile(r"\x1b\[[0-9;]*m")


class ConformanceFailure(RuntimeError):
    """A stable assertion that never exposes provider output or local paths."""


def refuse_external_authority() -> None:
    forbidden = [
        name
        for name in (
            "CLOUDFLARE_API_TOKEN",
            "CLOUDFLARE_ACCOUNT_ID",
            "CLOUDFLARE_API_KEY",
            "CLOUDFLARE_EMAIL",
            "DATABASE_URL",
        )
        if os.environ.get(name)
    ]
    if os.environ.get("FRAME_DEPLOYMENT") == "production":
        forbidden.append("FRAME_DEPLOYMENT")
    if forbidden:
        raise ConformanceFailure("local R2 conformance refused external authority variables")


def detect_wrangler(explicit: str | None) -> list[str]:
    command = (
        (["node", explicit] if explicit and explicit.endswith(".js") else [explicit])
        if explicit
        else ["npx", "--yes", f"wrangler@{WRANGLER_VERSION}"]
    )
    environment = os.environ.copy()
    environment.update(
        {
            "NO_COLOR": "1",
            "WRANGLER_LOG_PATH": "/tmp/frame-r2-wrangler-version.log",
            "WRANGLER_SEND_METRICS": "false",
        }
    )
    version = subprocess.run(
        [*command, "--version"],
        cwd=ROOT,
        env=environment,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    if version.returncode != 0 or ANSI.sub("", version.stdout).strip() != WRANGLER_VERSION:
        raise ConformanceFailure(f"Wrangler {WRANGLER_VERSION} is required")
    return command


def verify_checked_in_surface() -> None:
    config = CONFIG.read_text(encoding="utf-8")
    source = R2_SOURCE.read_text(encoding="utf-8")
    routing = ROUTING_SOURCE.read_text(encoding="utf-8")
    library = LIB_SOURCE.read_text(encoding="utf-8")
    for marker in (
        'binding = "RECORDINGS"',
        'bucket_name = "frame-recordings-local"',
        'FRAME_DEPLOYMENT = "local"',
    ):
        if marker not in config:
            raise ConformanceFailure("local R2 binding configuration drifted")
    for marker in (
        "impl ObjectStoreV1 for R2ObjectStoreV1",
        "Conditional {",
        "etag_does_not_match: Some(\"*\".into())",
        "StorageFailureKind::NotFound",
        "cross_tenant_not_found",
        "run_local_contract(&adapter)",
    ):
        if marker not in source:
            raise ConformanceFailure("R2 adapter contract surface drifted")
    if (
        f'path == "{CONFORMANCE_PATH}"' not in routing
        or "Route::LocalR2StorageConformance" not in routing
        or "config.production() || !valid_repository_conformance_target" not in library
    ):
        raise ConformanceFailure("loopback-only R2 conformance route drifted")


def reserve_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


class WorkerServer:
    def __init__(self, command: list[str], root: pathlib.Path) -> None:
        self.command = command
        self.root = root
        self.state = root / "state"
        self.state.mkdir(mode=0o700)
        self.port = reserve_port()
        self.log_path = root / "worker.log"
        self.process: subprocess.Popen[str] | None = None
        self.log_file: Any = None
        self.environment = os.environ.copy()
        self.environment.update(
            {
                "CI": "true",
                "NO_COLOR": "1",
                "WRANGLER_LOG_PATH": str(root / "wrangler.log"),
                "WRANGLER_SEND_METRICS": "false",
            }
        )

    def start(self) -> None:
        self.log_file = self.log_path.open("w", encoding="utf-8")
        self.process = subprocess.Popen(
            [
                *self.command,
                "dev",
                "--local",
                "--persist-to",
                str(self.state),
                "--config",
                str(CONFIG),
                "--ip",
                "127.0.0.1",
                "--port",
                str(self.port),
            ],
            cwd=ROOT,
            env=self.environment,
            stdin=subprocess.DEVNULL,
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
            text=True,
        )
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise ConformanceFailure("local Worker exited before becoming ready")
            try:
                status, _, _ = self.request("GET", "/health", timeout=1)
                if status == 200:
                    return
                raise ConformanceFailure("local Worker health response changed")
            except OSError:
                time.sleep(0.2)
        raise ConformanceFailure("local Worker did not become ready")

    def stop(self) -> None:
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        if self.log_file is not None:
            self.log_file.close()

    def request(
        self,
        method: str,
        path: str,
        *,
        host: str | None = None,
        timeout: float = 60,
    ) -> tuple[int, bytes, dict[str, str]]:
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
        headers = {"content-length": "0"}
        if host is not None:
            headers["host"] = host
        connection.request(method, path, headers=headers)
        response = connection.getresponse()
        raw = response.read()
        status = response.status
        response_headers = {key.lower(): value for key, value in response.getheaders()}
        connection.close()
        return status, raw, response_headers


def decode_contract(raw: bytes) -> dict[str, Any]:
    try:
        payload = json.loads(raw)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise ConformanceFailure("local R2 route returned invalid JSON") from error
    if not isinstance(payload, dict):
        raise ConformanceFailure("local R2 route response shape changed")
    return payload


def exercise_worker(server: WorkerServer) -> dict[str, Any]:
    status, raw, headers = server.request("POST", CONFORMANCE_PATH)
    if status != 200 or not headers.get("content-type", "").startswith("application/json"):
        raise ConformanceFailure("local R2 contract request failed")
    payload = decode_contract(raw)
    expected = {
        "schema_version": 1,
        "adapter": "cloudflare_r2_worker_binding_v1",
        "operations": ["put", "head", "get", "range", "copy", "delete", "list"],
        "conditions": [
            "immutable_create",
            "exact_replay",
            "version_match",
            "cross_tenant_not_found",
        ],
        "status": "passed",
    }
    if payload != expected:
        raise ConformanceFailure("local R2 adapter report changed")

    replay_status, replay_raw, _ = server.request("POST", CONFORMANCE_PATH)
    if replay_status != 200 or decode_contract(replay_raw) != expected:
        raise ConformanceFailure("local R2 adapter contract is not replay-safe")
    if server.request("GET", CONFORMANCE_PATH)[0] != 405:
        raise ConformanceFailure("local R2 route method guard changed")
    for lookalike in (
        f"{CONFORMANCE_PATH}/",
        f"{CONFORMANCE_PATH}%2f",
        f"{CONFORMANCE_PATH}/objects",
    ):
        if server.request("POST", lookalike)[0] != 404:
            raise ConformanceFailure("local R2 route accepted a path lookalike")
    if server.request(
        "POST", CONFORMANCE_PATH, host=f"localhost:{server.port}"
    )[0] != 404:
        raise ConformanceFailure("local R2 route accepted a non-IPv4-loopback authority")
    return payload


def digest_sources() -> str:
    digest = hashlib.sha256()
    for path in (RUNNER_SOURCE, CONFIG, R2_SOURCE, ROUTING_SOURCE, LIB_SOURCE):
        digest.update(path.relative_to(ROOT).as_posix().encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def write_evidence(path: pathlib.Path, payload: dict[str, Any]) -> None:
    report = {
        "schema_version": 1,
        "suite": "frame-r2-worker-binding-conformance",
        "runtime_boundary": "compiled_rust_wasm_worker_over_loopback_http",
        "storage": "isolated_wrangler_local_r2",
        "credential_mode": "none",
        "wrangler_version": WRANGLER_VERSION,
        "contract": payload,
        "scenarios": [
            "immutable_create_and_exact_replay",
            "head_get_and_bounded_range",
            "same_scope_conditional_copy",
            "provider_version_fenced_delete",
            "scoped_cursor_pagination",
            "cross_tenant_not_found_for_every_operation",
            "idempotent_delete_and_contract_replay",
            "exact_loopback_route_and_method_guard",
        ],
        "source_digest_sha256": digest_sources(),
        "result": "pass",
        "not_claimed": [
            "hosted_r2_behavior",
            "provider_network_or_quota_failures",
            "production_credentials_or_bucket_access",
            "durability_latency_residency_lifecycle_or_cost",
        ],
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wrangler-bin", help="direct path to pinned Wrangler 4.111.0")
    parser.add_argument(
        "--evidence",
        type=pathlib.Path,
        default=ROOT / "target" / "evidence" / "r2-storage-conformance.json",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        refuse_external_authority()
        verify_checked_in_surface()
        wrangler = detect_wrangler(arguments.wrangler_bin)
        with tempfile.TemporaryDirectory(prefix="frame-r2-conformance-") as directory:
            server = WorkerServer(wrangler, pathlib.Path(directory))
            try:
                server.start()
                payload = exercise_worker(server)
            finally:
                server.stop()
        write_evidence(arguments.evidence.resolve(), payload)
    except (ConformanceFailure, OSError, subprocess.SubprocessError, ValueError) as error:
        print(f"R2 storage conformance failed: {error}", file=sys.stderr)
        return 1
    print(
        "R2 Worker-binding conformance passed through compiled Worker "
        f"(credential-free local binding; Wrangler {WRANGLER_VERSION})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
