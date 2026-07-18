#!/usr/bin/env python3
"""Capture and inspect deterministic authenticated Leptos browser fixtures."""

from __future__ import annotations

import argparse
import base64
import hashlib
import importlib.util
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


ROOT = pathlib.Path(__file__).resolve().parents[2]
HELPER_PATH = ROOT / "scripts/ci/web-hydration-smoke.py"
SPEC = importlib.util.spec_from_file_location("frame_web_hydration_smoke", HELPER_PATH)
if SPEC is None or SPEC.loader is None:
    raise SystemExit("web authenticated browser: could not load DevTools helper")
HELPER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HELPER)
DevTools = HELPER.DevTools

PROFILE_CLEANUP_TIMEOUT_SECONDS = 5.0
PROFILE_CLEANUP_RETRY_SECONDS = 0.1


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"web authenticated browser: {message}")


def browser_executable() -> str:
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
    raise SystemExit("web authenticated browser: Chromium/Chrome is required")


def wait_ready(devtools: object) -> None:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if devtools.evaluate("document.readyState") == "complete":
            return
        time.sleep(0.05)
    raise SystemExit("web authenticated browser: document did not become ready")


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
                    "web authenticated browser: warning: Chrome profile cleanup "
                    f"remained incomplete after {PROFILE_CLEANUP_TIMEOUT_SECONDS:g}s: "
                    f"{error}",
                    file=sys.stderr,
                )
                return
            time.sleep(PROFILE_CLEANUP_RETRY_SECONDS)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--origin", default="http://127.0.0.1:3810")
    parser.add_argument("--evidence", type=pathlib.Path)
    parser.add_argument("--screenshots", type=pathlib.Path)
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
    browser = browser_executable()
    screenshots = args.screenshots or ROOT / "target/evidence/web-authenticated-screenshots"
    screenshots.mkdir(parents=True, exist_ok=True)
    fixtures = (
        ("dashboard-owner-desktop-dark", "/dashboard?fixture=owner&theme=dark", 1440, 1000, "dark", True),
        ("library-member-mobile-light", "/library?fixture=member&filter=ready&theme=light", 390, 844, "light", True),
        ("billing-admin-denied-tablet", "/billing?fixture=admin&theme=system", 768, 1024, "light", False),
        ("account-member-mobile-dark", "/settings/account?fixture=member&theme=dark", 390, 844, "dark", True),
        ("imports-admin-desktop-light", "/imports?fixture=admin&theme=light", 1440, 1000, "light", True),
        ("onboarding-member-tablet", "/onboarding?fixture=member&theme=system", 768, 1024, "dark", True),
    )
    port = HELPER.reserve_loopback_port()
    records: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory(
        prefix="frame-authenticated-chrome-", ignore_cleanup_errors=True
    ) as profile:
        process = subprocess.Popen(
            [
                browser,
                "--headless=new",
                "--disable-gpu",
                "--disable-dev-shm-usage",
                "--disable-setuid-sandbox",
                "--no-sandbox",
                "--no-first-run",
                "--no-default-browser-check",
                "--remote-debugging-address=127.0.0.1",
                f"--remote-debugging-port={port}",
                f"--user-data-dir={profile}",
                "about:blank",
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        devtools = None
        try:
            endpoint = f"http://127.0.0.1:{port}"
            # Hosted runners can spend several seconds starting crashpad and
            # the first browser process after a release build. Keep the probe
            # bounded, but do not turn normal runner variance into a false
            # product failure.
            deadline = time.monotonic() + 30
            while True:
                try:
                    with urllib.request.urlopen(f"{endpoint}/json/version", timeout=1):
                        break
                except (urllib.error.URLError, ConnectionError, TimeoutError):
                    require(process.poll() is None, "Chrome exited before DevTools was ready")
                    require(time.monotonic() < deadline, "Chrome DevTools did not start")
                    time.sleep(0.05)
            target_request = urllib.request.Request(
                f"{endpoint}/json/new?{urllib.parse.quote('about:blank', safe='')}",
                method="PUT",
            )
            with urllib.request.urlopen(target_request, timeout=5) as response:
                target = json.load(response)
            devtools = DevTools(target["webSocketDebuggerUrl"])
            for domain in ("Page", "Runtime", "Network", "Log"):
                devtools.command(f"{domain}.enable")

            for name, path, width, height, preferred, ready_expected in fixtures:
                devtools.command(
                    "Emulation.setDeviceMetricsOverride",
                    {
                        "width": width,
                        "height": height,
                        "deviceScaleFactor": 1,
                        "mobile": width < 600,
                    },
                )
                devtools.command(
                    "Emulation.setEmulatedMedia",
                    {
                        "media": "screen",
                        "features": [
                            {"name": "prefers-color-scheme", "value": preferred},
                            {"name": "prefers-reduced-motion", "value": "reduce"},
                        ],
                    },
                )
                devtools.events.clear()
                devtools.command("Page.navigate", {"url": f"{origin}{path}"})
                wait_ready(devtools)
                state = devtools.evaluate(
                    r"""(() => {
                      const main = document.querySelector('main');
                      const heading = document.querySelector('h1#page-title');
                      const nav = document.querySelector('nav[aria-label="Workspace"]');
                      const authenticatedRoots = document.querySelectorAll('#frame-authenticated-workspace-root');
                      const authenticatedRoot = authenticatedRoots[0];
                      const skip = document.querySelector('.skip-link');
                      const labels = new Map([...document.querySelectorAll('label[for]')].map(label => [label.htmlFor, label]));
                      const unlabeled = [...document.querySelectorAll('input:not([type="hidden"]), select, textarea')]
                        .filter(control => !control.getAttribute('aria-label') && !control.getAttribute('aria-labelledby') && !labels.has(control.id));
                      const ids = [...document.querySelectorAll('[id]')].map(node => node.id);
                      const duplicateIds = ids.filter((id, index) => ids.indexOf(id) !== index);
                      const emptyButtons = [...document.querySelectorAll('button')].filter(button => !button.textContent.trim() && !button.getAttribute('aria-label'));
                      const controls = [...document.querySelectorAll('button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled])')];
                      const undersized = controls.filter(control => {
                        const rect = control.getBoundingClientRect();
                        return rect.width < 44 || rect.height < 44;
                      });
                      skip.focus();
                      const skipStyle = getComputedStyle(skip);
                      const skipRect = skip.getBoundingClientRect();
                      const mainStyle = getComputedStyle(main);
                      const bodyStyle = getComputedStyle(document.body);
                      const parse = value => {
                        const values = value.match(/[\d.]+/g)?.map(Number) ?? [];
                        return values.length >= 3 ? [values[0], values[1], values[2], values[3] ?? 1] : null;
                      };
                      const composite = (front, back) => front[3] >= 1 ? front : [
                        front[0] * front[3] + back[0] * (1 - front[3]),
                        front[1] * front[3] + back[1] * (1 - front[3]),
                        front[2] * front[3] + back[2] * (1 - front[3]), 1
                      ];
                      const lum = color => {
                        const channels = color.slice(0, 3).map(value => value / 255).map(value => value <= .03928 ? value / 12.92 : ((value + .055) / 1.055) ** 2.4);
                        return .2126 * channels[0] + .7152 * channels[1] + .0722 * channels[2];
                      };
                      const foreground = parse(mainStyle.color);
                      const bodyBackground = parse(bodyStyle.backgroundColor);
                      const mainBackground = composite(parse(mainStyle.backgroundColor), bodyBackground);
                      const contrast = (Math.max(lum(foreground), lum(mainBackground)) + .05) / (Math.min(lum(foreground), lum(mainBackground)) + .05);
                      const effectiveBackground = element => {
                        const chain = [];
                        for (let node = element; node; node = node.parentElement) chain.push(node);
                        let background = [255, 255, 255, 1];
                        for (const node of chain.reverse()) {
                          const color = parse(getComputedStyle(node).backgroundColor);
                          if (color) background = composite(color, background);
                        }
                        return background;
                      };
                      const textContrasts = [...document.querySelectorAll('h1, h2, h3, p, a, button:not([disabled]), label, dt, dd, .role-badge, .state')]
                        .filter(element => element.textContent.trim() && element.getClientRects().length)
                        .map(element => {
                          const text = parse(getComputedStyle(element).color);
                          const background = effectiveBackground(element);
                          const renderedText = composite(text, background);
                          return {
                            value: (Math.max(lum(renderedText), lum(background)) + .05) / (Math.min(lum(renderedText), lum(background)) + .05),
                            element: `${element.tagName.toLowerCase()}.${element.className || ''}:${element.textContent.trim().slice(0, 48)}`,
                          };
                        });
                      const minimumText = textContrasts.reduce((minimum, candidate) => candidate.value < minimum.value ? candidate : minimum);
                      return {
                        title: document.title,
                        heading: heading?.textContent?.trim(),
                        readyState: document.readyState,
                        workspaceReady: document.body.textContent.includes('Local Frame workspace'),
                        accessDenied: document.body.textContent.includes('Access denied'),
                        horizontalOverflow: document.documentElement.scrollWidth > window.innerWidth + 1,
                        mainVisible: Boolean(main && main.getClientRects().length),
                        navVisible: Boolean(nav && nav.getClientRects().length),
                        authenticatedRootCount: authenticatedRoots.length,
                        browserLoaderEnabled: authenticatedRoot?.dataset.frameBrowserLoader === 'true',
                        currentLinks: nav ? nav.querySelectorAll('[aria-current="page"]').length : 0,
                        duplicateIds: duplicateIds.length,
                        unlabeledControls: unlabeled.length,
                        emptyButtons: emptyButtons.length,
                        undersizedControls: undersized.length,
                        skipTarget: skip?.getAttribute('href'),
                        skipVisible: skipRect.width > 0 && skipRect.height > 0,
                        skipOutlineWidth: parseFloat(skipStyle.outlineWidth),
                        contrast,
                        minimumTextContrast: minimumText.value,
                        minimumContrastElement: minimumText.element,
                        layoutColumns: document.querySelector('.workspace-layout') ? getComputedStyle(document.querySelector('.workspace-layout')).gridTemplateColumns : null,
                      };
                    })()"""
                )
                require(isinstance(state, dict), f"{name} browser state is invalid")
                require(state.get("readyState") == "complete", f"{name} did not load")
                require(bool(state.get("mainVisible")), f"{name} main is invisible")
                require(state.get("authenticatedRootCount") == 1, f"{name} browser boundary root drifted")
                require(not bool(state.get("browserLoaderEnabled")), f"{name} local fixture activated the production browser loader")
                require(not bool(state.get("horizontalOverflow")), f"{name} overflows horizontally")
                require(state.get("duplicateIds") == 0, f"{name} has duplicate IDs")
                require(state.get("unlabeledControls") == 0, f"{name} has unlabeled controls")
                require(state.get("emptyButtons") == 0, f"{name} has unnamed buttons")
                require(state.get("undersizedControls") == 0, f"{name} has an undersized form control")
                require(state.get("skipTarget") == "#main", f"{name} skip target drifted")
                require(bool(state.get("skipVisible")), f"{name} focused skip link is invisible")
                require(float(state.get("skipOutlineWidth", 0)) >= 3, f"{name} focus indicator is too small")
                require(float(state.get("contrast", 0)) >= 4.5, f"{name} body contrast is below WCAG AA")
                require(float(state.get("minimumTextContrast", 0)) >= 4.5, f"{name} visible text contrast is below WCAG AA: {state.get('minimumContrastElement')}={state.get('minimumTextContrast')}")
                require(bool(state.get("workspaceReady")) == ready_expected, f"{name} ready boundary drifted")
                require(bool(state.get("accessDenied")) != ready_expected, f"{name} denied boundary drifted")
                if ready_expected:
                    require(bool(state.get("navVisible")), f"{name} workspace nav is invisible")
                    require(state.get("currentLinks") == 1, f"{name} has no unique current nav link")

                screenshot = devtools.command(
                    "Page.captureScreenshot",
                    {"format": "png", "fromSurface": True, "captureBeyondViewport": False},
                )
                image = base64.b64decode(str(screenshot.get("data", "")), validate=True)
                require(image.startswith(b"\x89PNG\r\n\x1a\n") and len(image) > 1_000, f"{name} screenshot is invalid")
                screenshot_path = screenshots / f"{name}.png"
                screenshot_path.write_bytes(image)
                records.append(
                    {
                        "name": name,
                        "path": path,
                        "viewport": {"width": width, "height": height},
                        "preferred_color_scheme": preferred,
                        "screenshot": screenshot_path.name,
                        "screenshot_sha256": hashlib.sha256(image).hexdigest(),
                        "screenshot_bytes": len(image),
                        "heading": state.get("heading"),
                        "layout_columns": state.get("layoutColumns"),
                        "contrast": round(float(state.get("contrast", 0)), 3),
                        "minimum_text_contrast": round(float(state.get("minimumTextContrast", 0)), 3),
                        "minimum_contrast_element": state.get("minimumContrastElement"),
                    }
                )

            hashes = {record["screenshot_sha256"] for record in records}
            require(len(hashes) == len(records), "visual fixtures unexpectedly produced identical captures")
            diagnostics = []
            for event in devtools.events:
                params = event.get("params", {})
                if event.get("method") == "Runtime.exceptionThrown":
                    diagnostics.append(event)
                elif event.get("method") == "Runtime.consoleAPICalled" and isinstance(params, dict) and params.get("type") in {"assert", "error", "warning"}:
                    diagnostics.append(event)
            require(not diagnostics, f"browser console emitted diagnostics: {diagnostics[:2]}")
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

    evidence = {
        "schema": "frame.web-authenticated-browser-evidence.v1",
        "browser": pathlib.Path(browser).name,
        "fixture_count": len(records),
        "semantic_accessibility_scan": True,
        "keyboard_focus_scan": True,
        "responsive_overflow_scan": True,
        "data_free_browser_boundary_scan": True,
        "wcag_aa_body_contrast_scan": True,
        "visual_capture_diff_ready": True,
        "cross_browser_baselines_pending": True,
        "manual_screen_reader_pending": True,
        "fixtures": records,
    }
    rendered = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
