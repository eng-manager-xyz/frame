#!/usr/bin/env python3
"""Drive the release Leptos desktop UI by keyboard against the real Rust core."""

from __future__ import annotations

import argparse
import base64
import contextlib
import functools
import hashlib
import hmac
import importlib.util
import json
import os
import pathlib
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from collections.abc import Callable
from http import HTTPStatus
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from typing import NoReturn


ROOT = pathlib.Path(__file__).resolve().parents[2]
DIST = ROOT / "apps" / "desktop" / "ui" / "dist"
DEFAULT_HOST = ROOT / "target" / "release" / (
    "frame-desktop-e2e-host.exe" if os.name == "nt" else "frame-desktop-e2e-host"
)
HELPER_PATH = ROOT / "scripts" / "ci" / "web-hydration-smoke.py"
MAX_BRIDGE_REQUEST_BYTES = 70 * 1_024
WAIT_SECONDS = 15.0

HELPER_SPEC = importlib.util.spec_from_file_location(
    "frame_web_hydration_smoke", HELPER_PATH
)
if HELPER_SPEC is None or HELPER_SPEC.loader is None:
    raise RuntimeError("cannot load shared Chrome smoke helper")
HELPER = importlib.util.module_from_spec(HELPER_SPEC)
sys.modules[HELPER_SPEC.name] = HELPER
HELPER_SPEC.loader.exec_module(HELPER)


def fail(message: str) -> NoReturn:
    raise SystemExit(f"desktop browser journey failed: {message}")


def require(condition: bool, message: str) -> None:
    if not condition:
        fail(message)


def find_browser() -> str:
    for name in ("google-chrome", "chromium", "chromium-browser", "chrome"):
        executable = shutil.which(name)
        if executable:
            return executable
    for path in (
        pathlib.Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        pathlib.Path("/Applications/Chromium.app/Contents/MacOS/Chromium"),
        pathlib.Path("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
    ):
        if path.is_file():
            return str(path)
    fail("Chromium or Chrome is required")


class RustHost:
    """Serialized request/response bridge to one stateful Rust runtime."""

    def __init__(self, executable: pathlib.Path, root: pathlib.Path) -> None:
        environment = os.environ.copy()
        environment["FRAME_DESKTOP_E2E"] = "1"
        self.process = subprocess.Popen(
            [str(executable), "--root", str(root)],
            cwd=ROOT,
            env=environment,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self.lock = threading.Lock()
        self.last_snapshot: dict[str, object] | None = None
        self.dispatch_count = 0
        self.request_count = 0
        self.last_error: str | None = None

    def request(self, request: dict[str, object]) -> dict[str, object]:
        encoded = json.dumps(request, separators=(",", ":"))
        require(
            len(encoded.encode("utf-8")) <= MAX_BRIDGE_REQUEST_BYTES,
            "browser attempted an oversized native request",
        )
        with self.lock:
            require(self.process.poll() is None, self.failure_detail())
            require(
                self.process.stdin is not None and self.process.stdout is not None,
                "native host pipes are unavailable",
            )
            self.process.stdin.write(encoded + "\n")
            self.process.stdin.flush()
            response_line = self.process.stdout.readline()
            require(bool(response_line), self.failure_detail())
            try:
                response = json.loads(response_line)
            except json.JSONDecodeError:
                fail("native host returned malformed JSON")
            require(isinstance(response, dict), "native host response is not an object")
            self.request_count += 1
            if response.get("ok") is True and isinstance(response.get("value"), dict):
                self.last_error = None
                value = response["value"]
                if request.get("command") == "bootstrap_desktop":
                    snapshot = value.get("snapshot")
                    if isinstance(snapshot, dict):
                        self.last_snapshot = snapshot
                elif request.get("command") == "dispatch_main":
                    snapshot = value.get("snapshot")
                    if isinstance(snapshot, dict):
                        self.last_snapshot = snapshot
                    self.dispatch_count += 1
            else:
                error = response.get("error")
                self.last_error = error if isinstance(error, str) else "invalid_host_response"
            return response

    def snapshot(self) -> dict[str, object] | None:
        with self.lock:
            if self.last_snapshot is None:
                return None
            return json.loads(json.dumps(self.last_snapshot))

    def failure_detail(self) -> str:
        return_code = self.process.poll()
        return f"native Rust host exited unexpectedly with code {return_code}"

    def close(self) -> None:
        with self.lock:
            if self.process.stdin is not None:
                with contextlib.suppress(OSError):
                    self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        if self.process.returncode not in (0, None):
            stderr = ""
            if self.process.stderr is not None:
                stderr = self.process.stderr.read()[-2_000:]
            fail(f"native Rust host failed during shutdown: {stderr}")


class DesktopHandler(SimpleHTTPRequestHandler):
    """Closed static server plus one authenticated loopback invoke endpoint."""

    bridge: RustHost
    bridge_token: str
    inline_script_hash: str

    def end_headers(self) -> None:
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header(
            "Content-Security-Policy",
            "default-src 'self'; base-uri 'none'; object-src 'none'; "
            "connect-src 'self'; img-src 'self' data:; "
            f"script-src 'self' 'wasm-unsafe-eval' {self.inline_script_hash}; "
            "style-src 'self' 'unsafe-inline'",
        )
        super().end_headers()

    def do_GET(self) -> None:  # noqa: N802 - stdlib handler API
        if self.path == "/favicon.ico":
            self.send_response(HTTPStatus.NO_CONTENT)
            self.end_headers()
            return
        super().do_GET()

    def do_POST(self) -> None:  # noqa: N802 - stdlib handler API
        if self.path != "/__frame_e2e/invoke":
            self.send_error(HTTPStatus.NOT_FOUND)
            return
        token = self.headers.get("X-Frame-E2E-Token", "")
        if not hmac.compare_digest(token, self.bridge_token):
            self.send_error(HTTPStatus.FORBIDDEN)
            return
        try:
            length = int(self.headers.get("Content-Length", ""))
        except ValueError:
            self.send_error(HTTPStatus.BAD_REQUEST)
            return
        if not 0 < length <= MAX_BRIDGE_REQUEST_BYTES:
            self.send_error(HTTPStatus.REQUEST_ENTITY_TOO_LARGE)
            return
        try:
            payload = json.loads(self.rfile.read(length))
        except (json.JSONDecodeError, UnicodeDecodeError):
            self.send_error(HTTPStatus.BAD_REQUEST)
            return
        if not isinstance(payload, dict) or set(payload) != {"command", "args"}:
            self.send_error(HTTPStatus.BAD_REQUEST)
            return
        command = payload.get("command")
        arguments = payload.get("args")
        if not isinstance(arguments, dict):
            self.send_error(HTTPStatus.BAD_REQUEST)
            return
        if command == "bootstrap_main" and not arguments:
            request: dict[str, object] = {"command": "bootstrap_main"}
        elif command == "bootstrap_desktop" and not arguments:
            request = {"command": "bootstrap_desktop"}
        elif (
            command == "dispatch_main"
            and set(arguments) == {"requestJson"}
            and isinstance(arguments.get("requestJson"), str)
        ):
            request = {
                "command": "dispatch_main",
                "request_json": arguments["requestJson"],
            }
        else:
            request = {"command": "unsupported"}
        response = self.bridge.request(request)
        body = json.dumps(response, separators=(",", ":")).encode("utf-8")
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def wait_snapshot(
    bridge: RustHost,
    predicate: Callable[[dict[str, object]], bool],
    failure: str,
) -> dict[str, object]:
    deadline = time.monotonic() + WAIT_SECONDS
    value: dict[str, object] | None = None
    while time.monotonic() < deadline:
        value = bridge.snapshot()
        if value is not None and predicate(value):
            return value
        require(bridge.process.poll() is None, bridge.failure_detail())
        time.sleep(0.025)
    fail(
        f"{failure}; native_requests={bridge.request_count}; "
        f"Rust_dispatches={bridge.dispatch_count}; last_error={bridge.last_error}; "
        f"last snapshot={redacted_snapshot(value)}"
    )


def derive_inline_script_hash(distribution: pathlib.Path) -> str:
    index = (distribution / "index.html").read_text(encoding="utf-8")
    scripts = re.findall(r"<script\b[^>]*>(.*?)</script>", index, flags=re.DOTALL)
    require(len(scripts) == 1, "release desktop UI must contain one inline loader")
    digest = base64.b64encode(hashlib.sha256(scripts[0].encode("utf-8")).digest())
    return f"'sha256-{digest.decode('ascii')}'"


def redacted_snapshot(snapshot: dict[str, object] | None) -> dict[str, object] | None:
    if snapshot is None:
        return None
    return {
        key: snapshot.get(key)
        for key in (
            "operation_revision",
            "adapter",
            "recorder",
            "devices",
            "permission",
            "recovery",
            "editor",
            "export",
            "upload",
            "update",
            "announcement",
        )
    }


def state_is(snapshot: dict[str, object], key: str, expected: str) -> bool:
    value = snapshot.get(key)
    return isinstance(value, dict) and value.get("state") == expected


def dispatch_key(
    devtools: object,
    key: str,
    code: str,
    virtual_key: int,
    *,
    shift: bool = False,
) -> None:
    modifiers = 8 if shift else 0
    common = {
        "key": key,
        "code": code,
        "windowsVirtualKeyCode": virtual_key,
        "nativeVirtualKeyCode": virtual_key,
        "modifiers": modifiers,
    }
    key_down = {"type": "keyDown", **common}
    if key == "Enter":
        key_down["text"] = "\r"
    devtools.command("Input.dispatchKeyEvent", key_down)
    devtools.command("Input.dispatchKeyEvent", {"type": "keyUp", **common})


def active_descriptor(devtools: object) -> dict[str, object]:
    value = devtools.evaluate(
        r"""(() => {
          const node = document.activeElement;
          if (!node) return {};
          const label = (
            node.getAttribute('aria-label') ||
            node.innerText ||
            node.value ||
            node.textContent ||
            ''
          ).trim().replace(/\s+/g, ' ');
          return {
            tag: node.tagName.toLowerCase(),
            label,
            id: node.id || '',
            disabled: Boolean(node.disabled),
            visible: Boolean(node.getClientRects().length),
          };
        })()"""
    )
    require(isinstance(value, dict), "browser returned an invalid focus descriptor")
    return value


def activate_by_keyboard(
    devtools: object, label: str, focus_trace: list[dict[str, object]]
) -> None:
    deadline = time.monotonic() + WAIT_SECONDS
    while time.monotonic() < deadline:
        descriptor = active_descriptor(devtools)
        if (
            descriptor.get("tag") == "button"
            and descriptor.get("label") == label
            and descriptor.get("disabled") is False
            and descriptor.get("visible") is True
        ):
            focus_trace.append(descriptor)
            dispatch_key(devtools, "Enter", "Enter", 13)
            return
        dispatch_key(devtools, "Tab", "Tab", 9)
        time.sleep(0.015)
    fail(f"keyboard could not reach enabled control {label!r}")


def semantic_snapshot(devtools: object) -> dict[str, object]:
    value = devtools.evaluate(
        r"""(() => {
          const visible = node => Boolean(node && node.getClientRects().length);
          const ids = [...document.querySelectorAll('[id]')].map(node => node.id);
          const duplicateIds = [...new Set(ids.filter((id, index) => ids.indexOf(id) !== index))];
          const name = node => (
            node.getAttribute('aria-label') ||
            (node.getAttribute('aria-labelledby') || '').split(/\s+/)
              .map(id => document.getElementById(id)?.textContent || '').join(' ') ||
            (node.id ? document.querySelector(`label[for="${CSS.escape(node.id)}"]`)?.textContent : '') ||
            node.textContent ||
            ''
          ).trim().replace(/\s+/g, ' ');
          const controls = [...document.querySelectorAll('button,input,select,textarea')];
          const labelledBy = [...document.querySelectorAll('[aria-labelledby]')];
          return {
            duplicateIds,
            unnamedControls: controls.filter(node => visible(node) && !name(node))
              .map(node => `${node.tagName.toLowerCase()}#${node.id}`),
            brokenLabelReferences: labelledBy.filter(node =>
              node.getAttribute('aria-labelledby').split(/\s+/)
                .some(id => !document.getElementById(id))
            ).map(node => node.id || node.tagName.toLowerCase()),
            landmarks: {
              main: document.querySelectorAll('main').length,
              navigation: document.querySelectorAll('nav,[role="navigation"]').length,
              header: document.querySelectorAll('header').length,
            },
            liveRegions: document.querySelectorAll('[aria-live],[role="status"],[role="alert"]').length,
            progressIndicators: document.querySelectorAll('progress,meter').length,
            numericTimelineInputs: ['selection-start','selection-end','preview-position']
              .filter(id => document.getElementById(id)?.type === 'number').length,
            visibleEnabledButtons: controls.filter(node =>
              node.tagName === 'BUTTON' && visible(node) && !node.disabled
            ).length,
            backendStatusLive: document.querySelector('#backend-status')?.getAttribute('aria-live'),
            dialogCount: document.querySelectorAll('[role="alertdialog"]').length,
          };
        })()"""
    )
    require(isinstance(value, dict), "browser returned invalid accessibility semantics")
    return value


def run_journey(
    browser: str,
    origin: str,
    bridge_token: str,
    bridge: RustHost,
) -> tuple[list[dict[str, object]], dict[str, object], bool]:
    with tempfile.TemporaryDirectory(
        prefix="frame-desktop-chrome-", ignore_cleanup_errors=True
    ) as profile:
        profile_path = pathlib.Path(profile)
        port_file = profile_path / "DevToolsActivePort"
        process = subprocess.Popen(
            HELPER.chrome_launch_command(browser, profile_path),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        devtools = None
        try:
            port = HELPER.wait_for_devtools_port(process, port_file)
            target = HELPER.open_devtools_target(process, f"http://127.0.0.1:{port}")
            websocket_url = target.get("webSocketDebuggerUrl")
            require(isinstance(websocket_url, str), "Chrome target omitted its WebSocket URL")
            devtools = HELPER.DevTools(websocket_url)
            for domain in ("Page", "Runtime", "Log"):
                devtools.command(f"{domain}.enable")
            injection = (
                "Object.defineProperty(window,'__TAURI__',{configurable:false,"
                "value:{core:{invoke:async(command,args={})=>{"
                "const response=await fetch('/__frame_e2e/invoke',{method:'POST',"
                "headers:{'Content-Type':'application/json',"
                f"'X-Frame-E2E-Token':{json.dumps(bridge_token)}"
                "},body:JSON.stringify({command,args})});"
                "const payload=await response.json();"
                "if(!payload.ok)throw new Error(payload.error||'native_rejected');"
                "return payload.value;}}}});"
            )
            devtools.command(
                "Page.addScriptToEvaluateOnNewDocument", {"source": injection}
            )
            devtools.command("Page.navigate", {"url": origin})
            HELPER.wait_for_value(
                devtools,
                r"""(() => ({
                  ready: document.readyState === 'complete',
                  connected: document.body?.textContent?.includes('Backend connected') ?? false,
                  adapter: document.body?.textContent?.includes('Deterministic fake desktop backend ready.') ?? false,
                }))()""",
                lambda value: isinstance(value, dict)
                and value.get("ready")
                and value.get("connected")
                and value.get("adapter"),
                "desktop Leptos shell did not establish the Rust boundary",
            )
            focus_trace: list[dict[str, object]] = []

            activate_by_keyboard(devtools, "Studio", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state.get("recorder_configuration", {}).get("mode")
                == "studio",
                "Studio mode was not confirmed by Rust",
            )
            activate_by_keyboard(devtools, "Refresh capture targets", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state_is(state, "devices", "ready"),
                "device inventory did not become ready",
            )
            activate_by_keyboard(devtools, "Entire display", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state.get("selected_sources", {}).get("target") == "display",
                "display target selection was not confirmed",
            )
            activate_by_keyboard(devtools, "Confirm permissions", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state.get("permission") == "granted",
                "permission preparation did not complete",
            )
            activate_by_keyboard(devtools, "Start recording", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state_is(state, "recorder", "recording"),
                "recording did not start",
            )
            activate_by_keyboard(devtools, "Pause", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state_is(state, "recorder", "paused"),
                "recording did not pause",
            )
            activate_by_keyboard(devtools, "Resume", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state_is(state, "recorder", "recording"),
                "recording did not resume",
            )
            activate_by_keyboard(devtools, "Stop", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state_is(state, "recorder", "ready"),
                "recording did not stop",
            )
            activate_by_keyboard(devtools, "Scan for recovery", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state_is(state, "recovery", "available"),
                "recovery scan did not complete",
            )
            activate_by_keyboard(devtools, "Open sample recovery", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state_is(state, "recovery", "opened"),
                "sample recovery did not open",
            )
            activate_by_keyboard(devtools, "Open sample project", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state_is(state, "editor", "ready")
                and state.get("editor", {}).get("dirty") is False,
                "editor did not open",
            )
            activate_by_keyboard(devtools, "Trim to selection", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state.get("editor", {}).get("dirty") is True,
                "trim did not create a dirty revision",
            )
            activate_by_keyboard(devtools, "Save project", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state.get("editor", {}).get("dirty") is False
                and state.get("editor", {}).get("revision") == 2,
                "editor save did not commit revision two",
            )
            activate_by_keyboard(devtools, "Start export", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state_is(state, "export", "completed"),
                "fake export did not complete through Rust",
            )
            activate_by_keyboard(devtools, "Start upload", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state_is(state, "upload", "completed"),
                "fake upload did not complete through Rust",
            )
            activate_by_keyboard(devtools, "Toggle reduced motion", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state.get("settings", {}).get("reduced_motion") is True,
                "reduced-motion setting was not confirmed",
            )
            activate_by_keyboard(devtools, "Register global hotkeys", focus_trace)
            wait_snapshot(
                bridge,
                lambda state: state.get("lifecycle", {}).get("hotkeys_registered") is True,
                "hotkey registration was not confirmed",
            )
            for label, expected in (
                ("Check for updates", "available"),
                ("Install update", "ready_to_relaunch"),
                ("Relaunch Frame", "current"),
            ):
                activate_by_keyboard(devtools, label, focus_trace)
                wait_snapshot(
                    bridge,
                    lambda state, expected=expected: state_is(state, "update", expected),
                    f"update action {label!r} did not reach {expected}",
                )

            before_reverse = active_descriptor(devtools)
            dispatch_key(devtools, "Tab", "Tab", 9, shift=True)
            after_reverse = active_descriptor(devtools)
            reverse_focus = (
                after_reverse.get("visible") is True
                and after_reverse.get("label")
                and after_reverse != before_reverse
            )
            semantics = semantic_snapshot(devtools)
            diagnostics = []
            for event in devtools.events:
                method = event.get("method")
                if method == "Runtime.exceptionThrown":
                    diagnostics.append(event)
                elif method == "Log.entryAdded":
                    entry = event.get("params", {}).get("entry", {})
                    if isinstance(entry, dict) and entry.get("level") == "error":
                        diagnostics.append(event)
            require(not diagnostics, f"browser diagnostics were emitted: {diagnostics[:3]}")
            return focus_trace, semantics, bool(reverse_focus)
        finally:
            try:
                if devtools is not None:
                    devtools.close()
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)


def validate_semantics(
    focus_trace: list[dict[str, object]],
    semantics: dict[str, object],
    reverse_focus: bool,
) -> None:
    require(len(focus_trace) == 20, "not every essential action received keyboard focus")
    require(
        all(item.get("visible") is True and item.get("disabled") is False for item in focus_trace),
        "an essential keyboard action was hidden or disabled",
    )
    require(reverse_focus, "Shift+Tab did not move focus backward")
    require(semantics.get("duplicateIds") == [], "duplicate DOM ids were found")
    require(semantics.get("unnamedControls") == [], "visible controls lack accessible names")
    require(
        semantics.get("brokenLabelReferences") == [],
        "aria-labelledby contains broken references",
    )
    landmarks = semantics.get("landmarks")
    require(
        isinstance(landmarks, dict)
        and landmarks.get("main") == 1
        and int(landmarks.get("navigation", 0)) >= 1
        and landmarks.get("header") == 1,
        "desktop landmarks are incomplete",
    )
    require(int(semantics.get("liveRegions", 0)) >= 2, "live regions are incomplete")
    require(
        int(semantics.get("progressIndicators", 0)) >= 4,
        "meter and progress semantics are incomplete",
    )
    require(
        semantics.get("numericTimelineInputs") == 3,
        "numeric timeline alternative is incomplete",
    )
    require(
        semantics.get("backendStatusLive") == "polite",
        "backend status is not announced politely",
    )
    require(semantics.get("dialogCount") == 0, "an unexpected error dialog remained open")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", type=pathlib.Path, default=DEFAULT_HOST)
    parser.add_argument("--dist", type=pathlib.Path, default=DIST)
    parser.add_argument("--evidence", type=pathlib.Path, required=True)
    args = parser.parse_args()

    host_executable = args.host.resolve()
    distribution = args.dist.resolve()
    require(host_executable.is_file(), f"missing Rust E2E host {host_executable}")
    require(
        (distribution / "index.html").is_file(),
        f"missing release desktop UI {distribution}",
    )
    browser = find_browser()
    bridge_token = secrets.token_hex(32)
    inline_script_hash = derive_inline_script_hash(distribution)

    with tempfile.TemporaryDirectory(prefix="frame-desktop-e2e-root-") as native_root:
        bridge = RustHost(host_executable, pathlib.Path(native_root).resolve())
        handler = type(
            "BoundDesktopHandler",
            (DesktopHandler,),
            {
                "bridge": bridge,
                "bridge_token": bridge_token,
                "inline_script_hash": inline_script_hash,
            },
        )
        server = ThreadingHTTPServer(
            ("127.0.0.1", 0),
            functools.partial(handler, directory=str(distribution)),
        )
        server.daemon_threads = True
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            port = int(server.server_address[1])
            focus_trace, semantics, reverse_focus = run_journey(
                browser,
                f"http://127.0.0.1:{port}/",
                bridge_token,
                bridge,
            )
            validate_semantics(focus_trace, semantics, reverse_focus)
            final = wait_snapshot(
                bridge,
                lambda state: state_is(state, "export", "completed")
                and state_is(state, "upload", "completed")
                and state_is(state, "update", "current"),
                "final backend state is incomplete",
            )
            evidence = {
                "schema": "frame.desktop-browser-journey.v1",
                "backend": "real_rust_deterministic_fake",
                "browser": pathlib.Path(browser).name,
                "keyboard_only": True,
                "reverse_focus": reverse_focus,
                "essential_actions": [item["label"] for item in focus_trace],
                "dispatch_count": bridge.dispatch_count,
                "accessibility": semantics,
                "final_state": redacted_snapshot(final),
                "host_sha256": hashlib.sha256(host_executable.read_bytes()).hexdigest(),
                "ui_index_sha256": hashlib.sha256(
                    (distribution / "index.html").read_bytes()
                ).hexdigest(),
                "protected_assistive_technology_claimed": False,
                "protected_hardware_claimed": False,
            }
            output = args.evidence.resolve()
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(
                json.dumps(evidence, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            print(
                "desktop browser keyboard/accessibility journey passed "
                f"with {bridge.dispatch_count} Rust dispatches"
            )
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
            bridge.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
