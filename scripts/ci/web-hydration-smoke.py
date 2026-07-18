#!/usr/bin/env python3
"""Exercise SSR/no-JS and the production Wasm hydration module in Chromium."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import pathlib
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


PROFILE_CLEANUP_TIMEOUT_SECONDS = 5.0
PROFILE_CLEANUP_RETRY_SECONDS = 0.1


def fetch(origin: str, path: str) -> tuple[int, dict[str, str], bytes]:
    try:
        with urllib.request.urlopen(f"{origin}{path}", timeout=5) as response:
            return response.status, {key.lower(): value for key, value in response.headers.items()}, response.read()
    except urllib.error.HTTPError as error:
        return error.code, {key.lower(): value for key, value in error.headers.items()}, error.read()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"web hydration smoke: {message}")


def cleanup_browser_profile(profile: pathlib.Path) -> None:
    """Best-effort removal for profile files released just after Chrome exits."""
    deadline = time.monotonic() + PROFILE_CLEANUP_TIMEOUT_SECONDS
    while True:
        try:
            shutil.rmtree(profile)
            return
        except FileNotFoundError:
            return
        except OSError as error:
            if time.monotonic() >= deadline:
                print(
                    "web hydration smoke: warning: Chrome profile cleanup "
                    f"remained incomplete after {PROFILE_CLEANUP_TIMEOUT_SECONDS:g}s: "
                    f"{error}",
                    file=sys.stderr,
                )
                return
            time.sleep(PROFILE_CLEANUP_RETRY_SECONDS)


class DevTools:
    """Minimal RFC 6455 client for the Chrome DevTools Protocol."""

    def __init__(self, websocket_url: str) -> None:
        parsed = urllib.parse.urlsplit(websocket_url)
        require(parsed.scheme == "ws" and parsed.hostname is not None, "invalid DevTools URL")
        port = parsed.port or 80
        self.socket = socket.create_connection((parsed.hostname, port), timeout=5)
        self.buffer = b""
        key = base64.b64encode(os.urandom(16))
        path = parsed.path or "/"
        if parsed.query:
            path = f"{path}?{parsed.query}"
        request = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {parsed.hostname}:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key.decode('ascii')}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        ).encode("ascii")
        self.socket.sendall(request)
        response = b""
        while b"\r\n\r\n" not in response:
            response += self.socket.recv(4096)
        headers, self.buffer = response.split(b"\r\n\r\n", 1)
        require(headers.startswith(b"HTTP/1.1 101"), "DevTools WebSocket upgrade failed")
        expected = base64.b64encode(
            hashlib.sha1(key + b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest()
        )
        require(
            f"sec-websocket-accept: {expected.decode('ascii')}".lower()
            in headers.decode("ascii").lower(),
            "DevTools WebSocket accept key is invalid",
        )
        self.next_identifier = 0
        self.events: list[dict[str, object]] = []

    def close(self) -> None:
        self.socket.close()

    def _read_exact(self, length: int) -> bytes:
        while len(self.buffer) < length:
            chunk = self.socket.recv(max(4096, length - len(self.buffer)))
            require(bool(chunk), "DevTools WebSocket closed unexpectedly")
            self.buffer += chunk
        value, self.buffer = self.buffer[:length], self.buffer[length:]
        return value

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        mask = os.urandom(4)
        length = len(payload)
        if length < 126:
            header = struct.pack("!BB", 0x80 | opcode, 0x80 | length)
        elif length < 65_536:
            header = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, length)
        else:
            header = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, length)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self.socket.sendall(header + mask + masked)

    def _receive_text(self) -> str:
        fragments: list[bytes] = []
        initial_opcode: int | None = None
        while True:
            first, second = self._read_exact(2)
            final = bool(first & 0x80)
            opcode = first & 0x0F
            length = second & 0x7F
            if length == 126:
                length = struct.unpack("!H", self._read_exact(2))[0]
            elif length == 127:
                length = struct.unpack("!Q", self._read_exact(8))[0]
            mask = self._read_exact(4) if second & 0x80 else None
            payload = self._read_exact(length)
            if mask:
                payload = bytes(
                    byte ^ mask[index % 4] for index, byte in enumerate(payload)
                )
            if opcode == 0x8:
                raise SystemExit("web hydration smoke: DevTools WebSocket closed")
            if opcode == 0x9:
                self._send_frame(0xA, payload)
                continue
            if opcode not in (0x0, 0x1):
                continue
            if opcode == 0x1:
                initial_opcode = opcode
            fragments.append(payload)
            if final:
                require(initial_opcode == 0x1, "unexpected DevTools WebSocket fragment")
                return b"".join(fragments).decode("utf-8")

    def command(
        self, method: str, params: dict[str, object] | None = None
    ) -> dict[str, object]:
        self.next_identifier += 1
        identifier = self.next_identifier
        payload: dict[str, object] = {"id": identifier, "method": method}
        if params is not None:
            payload["params"] = params
        self._send_frame(0x1, json.dumps(payload).encode("utf-8"))
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            self.socket.settimeout(max(0.1, deadline - time.monotonic()))
            message = json.loads(self._receive_text())
            if message.get("id") != identifier:
                if isinstance(message, dict):
                    self.events.append(message)
                continue
            require("error" not in message, f"DevTools {method} failed: {message}")
            result = message.get("result", {})
            require(isinstance(result, dict), f"DevTools {method} returned invalid data")
            return result
        raise SystemExit(f"web hydration smoke: DevTools {method} timed out")

    def evaluate(self, expression: str) -> object:
        evaluation = self.command(
            "Runtime.evaluate",
            {
                "expression": expression,
                "returnByValue": True,
                "awaitPromise": True,
            },
        )
        require("exceptionDetails" not in evaluation, f"browser evaluation failed: {evaluation}")
        result = evaluation.get("result", {})
        require(isinstance(result, dict), "browser evaluation returned invalid data")
        return result.get("value")

    def wait_for_event(self, method: str) -> dict[str, object]:
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            for index, event in enumerate(self.events):
                if event.get("method") == method:
                    return self.events.pop(index)
            self.socket.settimeout(max(0.1, deadline - time.monotonic()))
            message = json.loads(self._receive_text())
            if isinstance(message, dict) and message.get("method") == method:
                return message
            if isinstance(message, dict):
                self.events.append(message)
        raise SystemExit(f"web hydration smoke: DevTools event timed out: {method}")


def wait_for_value(
    devtools: DevTools, expression: str, predicate: object, failure: str
) -> object:
    deadline = time.monotonic() + 10
    value: object = None
    while time.monotonic() < deadline:
        value = devtools.evaluate(expression)
        if predicate(value):
            return value
        time.sleep(0.05)
    diagnostics = [
        event
        for event in devtools.events
        if event.get("method")
        in {"Runtime.exceptionThrown", "Runtime.consoleAPICalled", "Log.entryAdded"}
    ]
    raise SystemExit(
        f"web hydration smoke: {failure}: {value}; diagnostics={diagnostics[:3]}"
    )


def reserve_loopback_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def chrome_interaction_smoke(browser: str, origin: str) -> dict[str, bool]:
    normal_state = r"""(() => {
      const status = document.querySelector('#frame-hydration-state');
      const help = document.querySelector('.player-keyboard-help');
      const button = help?.querySelector('button[aria-controls="player-keyboard-help-panel"]');
      const controls = [...(help?.querySelectorAll('.player-controls button') ?? [])];
      const rate = document.querySelector('#frame-playback-rate');
      const player = document.querySelector('#frame-public-player');
      const playerStatus = help?.querySelector('.player-status');
      const panel = document.querySelector('#player-keyboard-help-panel');
      const fallback = help?.querySelector('.player-keyboard-help-fallback');
      const collaboration = document.querySelector('.public-collaboration');
      const collaborationFallback = collaboration?.querySelector('.collaboration-fallback');
      const collaborationForm = collaboration?.querySelector('.comment-form');
      const collaborationStatus = collaboration?.querySelector('.collaboration-status');
      const visible = (node) => Boolean(node && !node.hidden &&
        getComputedStyle(node).display !== 'none' && node.getClientRects().length);
      return {
        ready: document.readyState === 'complete',
        hydrated: status?.dataset.frameHydrated === 'true',
        enhanced: help?.dataset.frameEnhanced === 'true',
        buttonVisible: visible(button),
        fallbackVisible: visible(fallback),
        collaborationEnhanced: collaboration?.dataset.frameEnhanced === 'true',
        collaborationFallbackVisible: visible(collaborationFallback),
        collaborationFormVisible: visible(collaborationForm),
        collaborationStatus: collaborationStatus?.textContent?.trim(),
        expanded: button?.getAttribute('aria-expanded'),
        panelHidden: panel?.hidden,
        panelVisible: visible(panel),
        buttonText: button?.textContent?.trim(),
        controlsVisible: controls.length === 6 && controls.every(visible),
        controlLabels: controls.map((control) => control.textContent?.trim()),
        rateVisible: visible(rate),
        fullscreenEnabled: controls.find((control) => control.textContent?.trim() === 'Fullscreen')?.disabled === false,
        pipEnabled: controls.find((control) => control.textContent?.trim() === 'Picture in picture')?.disabled === false,
        playerStatus: playerStatus?.textContent?.trim(),
        playerLabel: player?.getAttribute('aria-label'),
        playerControls: player?.hasAttribute('controls'),
        playerRemotePlaybackDisabled: player?.hasAttribute('disableremoteplayback'),
        statusText: status?.textContent?.trim(),
        probeAriaHidden: status?.getAttribute('aria-hidden'),
        probeRole: status?.getAttribute('role'),
        probeLive: status?.getAttribute('aria-live'),
        retainedTitle: document.body?.textContent?.includes('Local public recording') ?? false,
        retainedDescription: document.body?.textContent?.includes('A provider-neutral playback fixture for local UI checks.') ?? false,
        retainedPrivacy: document.body?.textContent?.includes('Analytics stay off unless a separate, same-share consent flow records a choice.') ?? false,
        retainedDescriptor: document.body?.textContent?.includes('Playback and caption paths come from a validated provider-neutral public descriptor.') ?? false,
        retainedTranscript: document.body?.textContent?.includes('English transcript (WebVTT)') ?? false,
        retainedComments: document.body?.textContent?.includes('Comments appear only after the same-origin collaboration service authorizes this exact share.') ?? false,
        privateSuccess: document.body?.textContent?.includes('Local Frame workspace') ?? false,
      };
    })()"""
    degraded_state = r"""(() => {
      const help = document.querySelector('.player-keyboard-help');
      const button = help?.querySelector('button[aria-controls="player-keyboard-help-panel"]');
      const fallback = help?.querySelector('.player-keyboard-help-fallback');
      const visible = (node) => Boolean(node && !node.hidden &&
        getComputedStyle(node).display !== 'none' && node.getClientRects().length);
      return {
        ready: document.readyState === 'complete',
        enhanced: help?.dataset.frameEnhanced === 'true',
        buttonVisible: visible(button),
        fallbackVisible: visible(fallback),
      };
    })()"""

    port = reserve_loopback_port()
    with tempfile.TemporaryDirectory(
        prefix="frame-chrome-", ignore_cleanup_errors=True
    ) as profile:
        process = subprocess.Popen(
            [
                browser,
                "--headless=new",
                "--disable-gpu",
                "--disable-dev-shm-usage",
                "--no-first-run",
                "--no-default-browser-check",
                f"--remote-debugging-port={port}",
                f"--user-data-dir={profile}",
                "about:blank",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        devtools: DevTools | None = None
        try:
            endpoint = f"http://127.0.0.1:{port}"
            deadline = time.monotonic() + 10
            while True:
                try:
                    with urllib.request.urlopen(f"{endpoint}/json/version", timeout=1):
                        break
                except (urllib.error.URLError, ConnectionError, TimeoutError):
                    require(process.poll() is None, "Chrome exited before DevTools was ready")
                    if time.monotonic() >= deadline:
                        raise SystemExit("web hydration smoke: Chrome DevTools did not start")
                    time.sleep(0.05)
            request = urllib.request.Request(
                f"{endpoint}/json/new?{urllib.parse.quote('about:blank', safe='')}",
                method="PUT",
            )
            with urllib.request.urlopen(request, timeout=5) as response:
                target = json.load(response)
            devtools = DevTools(target["webSocketDebuggerUrl"])
            devtools.command("Page.enable")
            devtools.command("Runtime.enable")
            devtools.command("Network.enable")
            devtools.command("DOM.enable")
            devtools.command("CSS.enable")
            devtools.command("Log.enable")
            devtools.command("Page.navigate", {"url": f"{origin}/s/fixture-public"})
            initial = wait_for_value(
                devtools,
                normal_state,
                lambda value: isinstance(value, dict)
                and value.get("ready")
                and value.get("hydrated")
                and value.get("enhanced")
                and value.get("collaborationEnhanced"),
                "interactive islands did not become ready",
            )
            require(initial["buttonVisible"], "player disclosure is not visible")
            require(initial["controlsVisible"], "hydrated accessible player controls are incomplete")
            require(initial["rateVisible"], "playback speed control is not visible")
            require(initial["fullscreenEnabled"], "approved fullscreen control is disabled")
            require(initial["pipEnabled"], "approved picture-in-picture control is disabled")
            require(initial["playerStatus"] == "Interactive player controls ready.", "player status did not hydrate")
            require(initial["playerLabel"] == "Video: Local public recording", "player accessible label drifted")
            require(initial["playerControls"], "native player controls are absent")
            require(initial["playerRemotePlaybackDisabled"], "remote playback is unexpectedly enabled")
            require(
                initial["controlLabels"] == [
                    "Play or pause",
                    "Back 10 seconds",
                    "Forward 10 seconds",
                    "Fullscreen",
                    "Picture in picture",
                    "Retry playback",
                ],
                "player control labels/order drifted",
            )
            require(not initial["fallbackVisible"], "static help remains duplicated after hydration")
            require(
                not initial["collaborationFallbackVisible"]
                and initial["collaborationFormVisible"],
                "collaboration enhancement did not replace its static fallback",
            )
            require(
                initial["collaborationStatus"]
                == "Interactive collaboration requires a live share.",
                "fixture collaboration did not fail closed",
            )
            require(initial["expanded"] == "false", "player disclosure starts expanded")
            require(initial["panelHidden"] and not initial["panelVisible"], "closed panel remains visible")
            require(initial["probeAriaHidden"] == "true", "hydration probe is exposed to assistive technology")
            require(initial["probeRole"] is None and initial["probeLive"] is None, "hydration probe creates a live announcement")
            require(initial["statusText"] == "Interactive enhancements ready.", "hydration probe did not advance")
            require(
                initial["retainedTitle"]
                and initial["retainedDescription"]
                and initial["retainedPrivacy"]
                and initial["retainedDescriptor"],
                "hydration replaced useful server-rendered content",
            )
            require(
                initial["retainedTranscript"] and initial["retainedComments"],
                "hydration removed collaboration or transcript fallback content",
            )
            require(not initial["privateSuccess"], "hydration inferred private success")

            point = devtools.evaluate(
                """(async () => { const button = document.querySelector('.player-keyboard-help button[aria-controls="player-keyboard-help-panel"]'); button.scrollIntoView({block: 'center', behavior: 'instant'}); await new Promise(requestAnimationFrame); const rect = button.getBoundingClientRect(); return {x: rect.left + rect.width / 2, y: rect.top + rect.height / 2}; })()"""
            )
            require(isinstance(point, dict), "player disclosure has no pointer target")
            devtools.command("Input.dispatchMouseEvent", {"type": "mouseMoved", **point})
            devtools.command(
                "Input.dispatchMouseEvent",
                {"type": "mousePressed", "button": "left", "clickCount": 1, **point},
            )
            devtools.command(
                "Input.dispatchMouseEvent",
                {"type": "mouseReleased", "button": "left", "clickCount": 1, **point},
            )
            opened = wait_for_value(
                devtools,
                normal_state,
                lambda value: isinstance(value, dict)
                and value.get("expanded") == "true"
                and not value.get("panelHidden")
                and value.get("panelVisible"),
                "pointer activation did not expose keyboard help",
            )
            require(opened["buttonText"] == "Hide shortcuts", "open disclosure has the wrong label")

            focused = devtools.evaluate(
                """(() => { const button = document.querySelector('.player-keyboard-help button[aria-controls="player-keyboard-help-panel"]'); button.focus(); return document.activeElement === button; })()"""
            )
            require(focused is True, "player disclosure cannot receive focus")
            key = {
                "key": "Enter",
                "code": "Enter",
                "windowsVirtualKeyCode": 13,
                "nativeVirtualKeyCode": 13,
            }
            devtools.command(
                "Input.dispatchKeyEvent", {"type": "keyDown", "text": "\r", **key}
            )
            devtools.command("Input.dispatchKeyEvent", {"type": "keyUp", **key})
            closed = wait_for_value(
                devtools,
                normal_state,
                lambda value: isinstance(value, dict)
                and value.get("expanded") == "false"
                and value.get("panelHidden")
                and not value.get("panelVisible"),
                "Enter did not close keyboard help",
            )
            require(closed["buttonText"] == "Show shortcuts", "closed disclosure has the wrong label")

            devtools.evaluate("new Promise(resolve => setTimeout(resolve, 100))")
            diagnostics = []
            expected_unbacked_fixture_media = {
                f"{origin}/api/v1/public/shares/fixture-public/media",
                f"{origin}/api/v1/public/shares/fixture-public/captions/en",
            }
            for event in devtools.events:
                method = event.get("method")
                params = event.get("params", {})
                if not isinstance(params, dict):
                    continue
                if method == "Runtime.exceptionThrown":
                    diagnostics.append(event)
                elif method == "Runtime.consoleAPICalled" and params.get("type") in {
                    "assert",
                    "error",
                    "warning",
                }:
                    diagnostics.append(event)
                elif method == "Log.entryAdded":
                    entry = params.get("entry", {})
                    if (
                        isinstance(entry, dict)
                        and entry.get("source") == "network"
                        and entry.get("url") in expected_unbacked_fixture_media
                    ):
                        continue
                    if isinstance(entry, dict) and entry.get("level") in {
                        "error",
                        "warning",
                    }:
                        diagnostics.append(event)
            require(not diagnostics, f"browser console emitted diagnostics: {diagnostics[:2]}")
            devtools.events.clear()

            devtools.command("Network.setCacheDisabled", {"cacheDisabled": True})
            devtools.command("Network.setBlockedURLs", {"urls": ["*://*/assets/*"]})
            devtools.command("Page.reload", {"ignoreCache": True})
            degraded = wait_for_value(
                devtools,
                degraded_state,
                lambda value: isinstance(value, dict)
                and value.get("ready")
                and not value.get("enhanced"),
                "asset-blocked page did not settle into SSR mode",
            )
            require(degraded["fallbackVisible"], "static keyboard help disappears when assets fail")
            require(not degraded["buttonVisible"], "inactive disclosure is visible when assets fail")

            # Exercise a genuinely JavaScript-disabled document, then use the
            # DevTools DOM/CSS domains (which do not execute page script) to
            # prove the static help is rendered and the inactive control is
            # absent from the visual UI.
            devtools.command("Network.setBlockedURLs", {"urls": []})
            devtools.command("Emulation.setScriptExecutionDisabled", {"value": True})
            devtools.events.clear()
            devtools.command("Page.reload", {"ignoreCache": True})
            devtools.wait_for_event("Page.loadEventFired")
            deadline = time.monotonic() + 10
            document_node = 0
            fallback_node = 0
            button_node = 0
            disabled_html = ""
            while time.monotonic() < deadline:
                document = devtools.command("DOM.getDocument", {"depth": -1})
                root = document.get("root", {})
                if not isinstance(root, dict):
                    time.sleep(0.05)
                    continue
                document_node = int(root.get("nodeId", 0))
                if not document_node:
                    time.sleep(0.05)
                    continue
                outer = devtools.command("DOM.getOuterHTML", {"nodeId": document_node})
                disabled_html = str(outer.get("outerHTML", ""))
                fallback = devtools.command(
                    "DOM.querySelector",
                    {
                        "nodeId": document_node,
                        "selector": ".player-keyboard-help-fallback",
                    },
                )
                button = devtools.command(
                    "DOM.querySelector",
                    {
                        "nodeId": document_node,
                        "selector": ".player-keyboard-help button[aria-controls=\"player-keyboard-help-panel\"]",
                    },
                )
                fallback_node = int(fallback.get("nodeId", 0))
                button_node = int(button.get("nodeId", 0))
                if (
                    fallback_node
                    and button_node
                    and 'data-frame-enhanced="true"' not in disabled_html
                ):
                    break
                time.sleep(0.05)
            require(document_node and fallback_node and button_node, "JavaScript-disabled player help did not render")
            fallback_style_result = devtools.command(
                "CSS.getComputedStyleForNode", {"nodeId": fallback_node}
            )
            button_style_result = devtools.command(
                "CSS.getComputedStyleForNode", {"nodeId": button_node}
            )
            fallback_style = {
                item["name"]: item["value"]
                for item in fallback_style_result.get("computedStyle", [])
                if isinstance(item, dict) and "name" in item and "value" in item
            }
            button_style = {
                item["name"]: item["value"]
                for item in button_style_result.get("computedStyle", [])
                if isinstance(item, dict) and "name" in item and "value" in item
            }
            require(
                fallback_style.get("display") != "none"
                and fallback_style.get("visibility") != "hidden",
                "JavaScript-disabled static keyboard help is not visible",
            )
            require(
                button_style.get("display") == "none",
                "JavaScript-disabled disclosure exposes an inactive control",
            )
            return {
                "pointer_activation": True,
                "keyboard_activation": True,
                "focus_verified": True,
                "degraded_asset_fallback": True,
                "javascript_disabled_browser_fallback": True,
                "probe_aria_hidden": True,
                "ssr_content_preserved": True,
                "accessible_player_controls": True,
                "zero_hydration_console_diagnostics": True,
            }
        finally:
            try:
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
            finally:
                cleanup_browser_profile(pathlib.Path(profile))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--origin", default="http://127.0.0.1:3810")
    parser.add_argument("--evidence", type=pathlib.Path)
    args = parser.parse_args()
    parsed_origin = urllib.parse.urlsplit(args.origin.rstrip("/"))
    require(
        parsed_origin.scheme == "http"
        and parsed_origin.hostname in {"127.0.0.1", "::1"}
        and parsed_origin.username is None
        and parsed_origin.password is None
        and parsed_origin.path in ("", "/")
        and not parsed_origin.query
        and not parsed_origin.fragment,
        "--origin must be an exact loopback HTTP origin",
    )
    try:
        origin_port = parsed_origin.port
    except ValueError:
        origin_port = None
    require(origin_port is not None, "--origin must include a loopback port")
    origin_host = f"[{parsed_origin.hostname}]" if parsed_origin.hostname == "::1" else parsed_origin.hostname
    origin = f"http://{origin_host}:{origin_port}"

    landing_status, _, landing_bytes = fetch(origin, "/")
    unavailable_status, _, unavailable_bytes = fetch(origin, "/s/not-a-fixture")
    processing_status, _, processing_bytes = fetch(origin, "/s/fixture-processing")
    private_status, _, private_bytes = fetch(origin, "/dashboard")
    public_status, public_headers, public_bytes = fetch(origin, "/s/fixture-public")
    landing = landing_bytes.decode("utf-8")
    unavailable = unavailable_bytes.decode("utf-8")
    processing = processing_bytes.decode("utf-8")
    private = private_bytes.decode("utf-8")
    public = public_bytes.decode("utf-8")
    require(landing_status == 200, f"landing returned {landing_status}")
    require(unavailable_status == 404, f"unavailable share returned {unavailable_status}")
    require(processing_status == 202, f"processing share returned {processing_status}")
    require(private_status == 401, f"private shell returned {private_status}")
    require(public_status == 200, f"public fixture returned {public_status}")
    require("Record locally. Share deliberately." in landing, "no-JS landing is empty")
    require('rel="canonical"' in landing, "landing metadata is absent")
    require("Recording unavailable" in unavailable, "no-JS unavailable state is absent")
    require("not-a-fixture" not in unavailable, "unavailable state leaks the identifier")
    require("Recording processing" in processing and "<video" not in processing, "processing state is not fail-closed")
    require("Sign in required" in private and "Local Frame workspace" not in private, "private shell leaked before session bootstrap")
    require(
        "<video" in public
        and 'id="frame-public-player"' in public
        and "player-keyboard-help-fallback" in public
        and "collaboration-fallback" in public
        and "English transcript (WebVTT)" in public
        and "Comments appear only after" in public,
        "public static player fallback is incomplete",
    )
    require(public_headers.get("cache-control") == "no-store", "public HTML can outlive its hashed asset closure")
    require("Server-rendered content ready." in public, "SSR hydration boundary is absent")
    require(public.count('data-frame-hydration-scope="interaction-island"') == 3, "unexpected hydration scope")
    require(public_headers.get("content-security-policy", "").find("script-src 'self' 'wasm-unsafe-eval'") >= 0, "CSP does not admit only same-origin Wasm")
    asset_paths = re.findall(r'(?:src|href)="(/assets/[a-z0-9_-]+-[0-9a-f]{64}\.(?:js|wasm))"', public)
    require(len(set(asset_paths)) == 2, "SSR does not reference exactly the hashed loader closure")
    for asset_path in sorted(set(asset_paths)):
        asset_status, asset_headers, asset_body = fetch(origin, asset_path)
        require(asset_status == 200 and asset_body, f"asset unavailable: {asset_path}")
        require(asset_headers.get("cache-control") == "public, max-age=31536000, immutable", f"asset is not immutable: {asset_path}")

    browser = next(
        (
            executable
            for name in ("google-chrome", "chromium", "chromium-browser", "chrome")
            if (executable := shutil.which(name))
        ),
        None,
    )
    if browser is None:
        browser = next(
            (
                str(path)
                for path in (
                    pathlib.Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
                    pathlib.Path("/Applications/Chromium.app/Contents/MacOS/Chromium"),
                    pathlib.Path("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
                )
                if path.is_file()
            ),
            None,
        )
    require(browser is not None, "Chromium/Chrome is required for the release smoke")
    interaction = chrome_interaction_smoke(browser, origin)

    evidence = {
        "schema": "frame.web-hydration-smoke.v1",
        "origin": origin,
        "browser": pathlib.Path(browser).name,
        "ssr": True,
        "no_javascript_fallback": True,
        "hydrated": True,
        "hydration_scope": "interaction-islands-v1",
        "content_fingerprinted": True,
        "immutable_assets": True,
        "public_html_no_store": True,
        "private_success_inferred": False,
        **interaction,
    }
    encoded = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(encoded, encoding="utf-8")
    print(encoded, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
