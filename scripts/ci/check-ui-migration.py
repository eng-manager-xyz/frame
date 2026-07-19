#!/usr/bin/env python3
"""Keep every Leptos product surface on the shared frame-ui primitives."""

from __future__ import annotations

import pathlib
import re


ROOT = pathlib.Path(__file__).resolve().parents[2]
APPLICATION_SOURCES = (
    ROOT / "apps" / "web" / "src" / "pages.rs",
    ROOT / "apps" / "web" / "src" / "hydration.rs",
    ROOT / "apps" / "desktop" / "ui" / "src" / "main.rs",
)
STATIC_UI_SOURCES = (
    ROOT / "apps" / "control-plane" / "src" / "legacy_desktop_session_web_runtime.rs",
    ROOT / "apps" / "control-plane" / "src" / "legacy_extension_auth_web_runtime.rs",
    ROOT / "apps" / "control-plane" / "src" / "legacy_protected_integrations_web_runtime.rs",
)
RAW_PRIMITIVE = re.compile(
    r"<(?:button|input|select|textarea|progress|meter|label|nav|fieldset)(?:\s|>)"
)
REQUIRED_COMPONENTS = (
    "Alert",
    "Badge",
    "Button",
    "ButtonGroup",
    "Card",
    "Input",
    "Label",
    "NavigationMenu",
    "Progress",
    "Select",
)


def main() -> int:
    failures: list[str] = []
    combined = ""
    for path in APPLICATION_SOURCES:
        source = path.read_text(encoding="utf-8")
        combined += source
        for number, line in enumerate(source.splitlines(), start=1):
            if RAW_PRIMITIVE.search(line) and "contains(\"<progress" not in line:
                failures.append(
                    f"{path.relative_to(ROOT)}:{number}: raw UI primitive bypasses frame-ui"
                )

    for component in REQUIRED_COMPONENTS:
        if f"<{component}" not in combined:
            failures.append(f"shared component is no longer used: {component}")

    for path in STATIC_UI_SOURCES:
        source = path.read_text(encoding="utf-8")
        if "frame_ui::class_contract" not in source:
            failures.append(
                f"{path.relative_to(ROOT)}: static utility page bypasses frame-ui contract"
            )
        if "<style" in source:
            failures.append(
                f"{path.relative_to(ROOT)}: static utility page owns local CSS"
            )

    control_plane_manifest = (
        ROOT / "apps" / "control-plane" / "Cargo.toml"
    ).read_text(encoding="utf-8")
    if 'frame-ui = { path = "../../crates/ui", default-features = false }' not in control_plane_manifest:
        failures.append("control-plane must consume frame-ui without linking Leptos")
    if "leptos.workspace = true" in control_plane_manifest:
        failures.append("control-plane must not link the incompatible Leptos Wasm runtime")

    forbidden = {
        ROOT / "apps" / "desktop" / "ui" / "src" / "app.css":
            "legacy desktop stylesheet still exists",
    }
    for path, message in forbidden.items():
        if path.exists():
            failures.append(message)

    web_source = APPLICATION_SOURCES[0].read_text(encoding="utf-8")
    if "const STYLE" in web_source:
        failures.append("legacy web STYLE constant still exists")

    desktop_html = (ROOT / "apps" / "desktop" / "ui" / "index.html").read_text(
        encoding="utf-8"
    )
    if 'data-trunk rel="css"' in desktop_html:
        failures.append("desktop still loads a separate Trunk CSS asset")

    for path in (ROOT / "crates" / "application" / "src").glob("*.rs"):
        if re.search(r"<!doctype|<!DOCTYPE|<html(?:\\s|>)", path.read_text(encoding="utf-8")):
            failures.append(
                f"{path.relative_to(ROOT)}: application crate still owns UI markup"
            )

    if failures:
        raise SystemExit("\n".join(failures))

    print("verified all Leptos product controls use frame-ui primitives")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
