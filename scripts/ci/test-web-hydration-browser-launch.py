#!/usr/bin/env python3
"""Deterministic tests for the bounded hydration-smoke browser launcher."""

from __future__ import annotations

import importlib.util
import io
import pathlib
import tempfile
from collections.abc import Callable


ROOT = pathlib.Path(__file__).resolve().parents[2]
HELPER_PATH = ROOT / "scripts/ci/web-hydration-smoke.py"
SPEC = importlib.util.spec_from_file_location("frame_web_hydration_smoke", HELPER_PATH)
if SPEC is None or SPEC.loader is None:
    raise SystemExit("browser launch test: could not load hydration smoke helper")
HELPER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HELPER)


class FakeProcess:
    def __init__(self, return_code: int | None = None) -> None:
        self.return_code = return_code

    def poll(self) -> int | None:
        return self.return_code


class FakeResponse(io.BytesIO):
    def __enter__(self) -> FakeResponse:
        return self

    def __exit__(self, *_args: object) -> None:
        self.close()


def expect_failure(call: Callable[[], object], message: str) -> None:
    try:
        call()
    except SystemExit as error:
        if str(error) != message:
            raise AssertionError(f"unexpected failure: {error}") from error
        return
    raise AssertionError(f"expected failure: {message}")


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-browser-launch-test-") as temporary:
        profile = pathlib.Path(temporary)
        command = HELPER.chrome_launch_command("google-chrome", profile)
        expected_flags = {
            "--headless=new",
            "--disable-dev-shm-usage",
            "--remote-debugging-address=127.0.0.1",
            "--remote-debugging-port=0",
            f"--user-data-dir={profile}",
        }
        if command[0] != "google-chrome" or command[-1] != "about:blank":
            raise AssertionError(f"unexpected Chrome command boundary: {command}")
        if not expected_flags.issubset(command):
            raise AssertionError(f"Chrome command is missing fixed flags: {command}")

        port_file = profile / "DevToolsActivePort"
        port_file.write_text("45123\n/devtools/browser/test\n", encoding="utf-8")
        port = HELPER.wait_for_devtools_port(FakeProcess(), port_file)
        if port != 45_123:
            raise AssertionError(f"unexpected DevTools port: {port}")

        port_file.unlink()
        ticks = iter((0.0, 29.9, 30.0))
        expect_failure(
            lambda: HELPER.wait_for_devtools_port(
                FakeProcess(),
                port_file,
                monotonic=lambda: next(ticks),
                pause=lambda _seconds: None,
            ),
            "web hydration smoke: Chrome DevTools did not start within 30 seconds",
        )
        expect_failure(
            lambda: HELPER.wait_for_devtools_port(
                FakeProcess(1),
                port_file,
                monotonic=lambda: 0.0,
                pause=lambda _seconds: None,
            ),
            "web hydration smoke: Chrome exited before DevTools was ready",
        )

        calls: list[tuple[object, float]] = []
        responses = iter(
            (
                TimeoutError("port file preceded HTTP readiness"),
                FakeResponse(b'{"Browser":"Chrome/150"}'),
                FakeResponse(
                    b'{"webSocketDebuggerUrl":"ws://127.0.0.1:45123/devtools/page/1"}'
                ),
            )
        )

        def late_opener(request: object, timeout: float) -> FakeResponse:
            calls.append((request, timeout))
            response = next(responses)
            if isinstance(response, BaseException):
                raise response
            return response

        ticks = iter((0.0, 1.0))
        target = HELPER.open_devtools_target(
            FakeProcess(),
            "http://127.0.0.1:45123",
            monotonic=lambda: next(ticks),
            pause=lambda _seconds: None,
            opener=late_opener,
        )
        if target.get("webSocketDebuggerUrl") != (
            "ws://127.0.0.1:45123/devtools/page/1"
        ):
            raise AssertionError(f"unexpected DevTools target: {target}")
        if len(calls) != 3 or any(timeout != 2 for _, timeout in calls):
            raise AssertionError(f"DevTools probes were not bounded: {calls}")
        target_request = calls[-1][0]
        if not isinstance(target_request, HELPER.urllib.request.Request):
            raise AssertionError("DevTools target creation did not use an HTTP request")
        if target_request.get_method() != "PUT":
            raise AssertionError("DevTools target creation must use PUT")

        ticks = iter((0.0, 30.0))
        expect_failure(
            lambda: HELPER.open_devtools_target(
                FakeProcess(),
                "http://127.0.0.1:45123",
                monotonic=lambda: next(ticks),
                pause=lambda _seconds: None,
                opener=lambda _request, timeout: (_ for _ in ()).throw(
                    TimeoutError(f"bounded at {timeout}")
                ),
            ),
            "web hydration smoke: Chrome DevTools HTTP endpoint did not become "
            "ready within 30 seconds",
        )
        expect_failure(
            lambda: HELPER.open_devtools_target(
                FakeProcess(1),
                "http://127.0.0.1:45123",
                monotonic=lambda: 0.0,
                pause=lambda _seconds: None,
            ),
            "web hydration smoke: Chrome exited before DevTools was ready",
        )

    print("web hydration browser launch tests passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
