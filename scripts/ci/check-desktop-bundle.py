#!/usr/bin/env python3
"""Fail closed when the generated desktop WebView bundle is stale or unsafe."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import plistlib
import re
import struct
from pathlib import Path
from typing import NoReturn


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_DIST = ROOT / "apps" / "desktop" / "ui" / "dist"
ICON_PNG = ROOT / "apps" / "desktop" / "icons" / "icon.png"
ICON_ICO = ROOT / "apps" / "desktop" / "icons" / "icon.ico"
MACOS_INFO_PLIST = ROOT / "apps" / "desktop" / "Info.plist"
MACOS_SIGNER = ROOT / "scripts" / "ci" / "sign-macos-local-app.sh"
DESKTOP_MAIN = ROOT / "apps" / "desktop" / "src" / "main.rs"
# Generated with ImageMagick 7.1.1-47 from the checked-in PNG:
# magick icon.png -background none -alpha on -filter Lanczos \
#   -define icon:auto-resize=256,128,64,48,32,24,16 icon.ico
ICON_PNG_SHA256 = "c5f0a61b791517ed943a34a983720ab16ad41da01fbfb91d091a4323209d9c89"
ICON_ICO_SHA256 = "ceb59ddf0620207d438b50258716bae32c964e8d3176b9405d7818c3df7900a8"
ICON_SIZES = (256, 128, 64, 48, 32, 24, 16)


def fail(message: str) -> NoReturn:
    raise SystemExit(f"desktop bundle check failed: {message}")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def one(paths: list[Path], label: str) -> Path:
    if len(paths) != 1:
        fail(f"expected exactly one {label}, found {len(paths)}")
    return paths[0]


def png_dimensions(payload: bytes, label: str) -> tuple[int, int]:
    if (
        len(payload) < 33
        or payload[:8] != b"\x89PNG\r\n\x1a\n"
        or struct.unpack_from(">I", payload, 8)[0] != 13
        or payload[12:16] != b"IHDR"
    ):
        fail(f"{label} is not a canonical PNG")
    width, height, depth, color, compression, filtering, interlace = struct.unpack_from(
        ">IIBBBBB", payload, 16
    )
    if (depth, color, compression, filtering, interlace) != (8, 6, 0, 0, 0):
        fail(f"{label} is not an 8-bit non-interlaced RGBA PNG")
    return width, height


def validate_icon_contract(tauri: dict[str, object]) -> None:
    if tauri.get("bundle", {}).get("icon") != ["icons/icon.png", "icons/icon.ico"]:
        fail("Tauri bundle does not declare the PNG and Windows ICO")
    try:
        png = ICON_PNG.read_bytes()
        ico = ICON_ICO.read_bytes()
    except OSError as error:
        fail(f"desktop icon is unavailable: {error}")
    if hashlib.sha256(png).hexdigest() != ICON_PNG_SHA256:
        fail("checked-in Frame PNG drifted without regenerating the icon contract")
    if hashlib.sha256(ico).hexdigest() != ICON_ICO_SHA256:
        fail("checked-in Windows ICO is absent or not the deterministic Frame artifact")
    if png_dimensions(png, "Frame source icon") != (512, 512):
        fail("Frame source icon must remain 512x512")

    if len(ico) < 6 or struct.unpack_from("<HHH", ico) != (0, 1, len(ICON_SIZES)):
        fail("Windows ICO header or image count is invalid")
    directory_end = 6 + 16 * len(ICON_SIZES)
    expected_offset = directory_end
    for index, expected_size in enumerate(ICON_SIZES):
        width_byte, height_byte, colors, reserved, planes, bits, size, offset = (
            struct.unpack_from("<BBBBHHII", ico, 6 + 16 * index)
        )
        width = width_byte or 256
        height = height_byte or 256
        if (
            (width, height) != (expected_size, expected_size)
            or colors != 0
            or reserved != 0
            or planes != 1
            or bits != 32
            or offset != expected_offset
            or size < 40
            or offset + size > len(ico)
        ):
            fail(f"Windows ICO directory entry {index} is invalid")
        payload = ico[offset : offset + size]
        if expected_size == 256:
            if png_dimensions(payload, "256px Windows icon") != (256, 256):
                fail("Windows ICO 256px PNG dimensions drifted")
        else:
            if len(payload) < 40:
                fail(f"Windows ICO {expected_size}px bitmap is truncated")
            header = struct.unpack_from("<IiiHHIIiiII", payload)
            dib_size, dib_width, dib_height, dib_planes, dib_bits, compression = header[:6]
            image_bytes = header[6]
            mask_bytes = ((expected_size + 31) // 32) * 4 * expected_size
            if (
                dib_size != 40
                or dib_width != expected_size
                or dib_height != expected_size * 2
                or dib_planes != 1
                or dib_bits != 32
                or compression != 0
                or image_bytes != expected_size * expected_size * 4
                or len(payload) != 40 + image_bytes + mask_bytes
            ):
                fail(f"Windows ICO {expected_size}px bitmap contract drifted")
        expected_offset += size
    if expected_offset != len(ico):
        fail("Windows ICO has trailing or overlapping payload data")


def validate_macos_capture_contract(tauri: dict[str, object]) -> None:
    bundle = tauri.get("bundle")
    if not isinstance(bundle, dict):
        fail("Tauri bundle configuration is absent")
    macos = bundle.get("macOS")
    if not isinstance(macos, dict):
        fail("macOS bundle configuration is absent")
    if macos.get("minimumSystemVersion") != "13.0":
        fail("ScreenCaptureKit system-audio builds must require macOS 13.0 or newer")
    if macos.get("infoPlist") != "Info.plist":
        fail("macOS bundle does not merge the checked-in privacy plist")
    try:
        with MACOS_INFO_PLIST.open("rb") as source:
            info = plistlib.load(source)
    except (OSError, plistlib.InvalidFileException) as error:
        fail(f"macOS privacy plist is unavailable: {error}")
    purpose = info.get("NSScreenCaptureUsageDescription")
    if not isinstance(purpose, str) or len(purpose.strip()) < 24:
        fail("macOS screen-capture purpose text is absent or too vague")
    expected_identity = {
        "CFBundleIdentifier": tauri.get("identifier"),
        "CFBundleName": tauri.get("productName"),
        "CFBundleShortVersionString": tauri.get("version"),
        "CFBundleVersion": tauri.get("version"),
        "LSMinimumSystemVersion": macos.get("minimumSystemVersion"),
    }
    for key, expected in expected_identity.items():
        if info.get(key) != expected:
            fail(f"macOS privacy plist {key} drifted from Tauri configuration")
    downloads_purpose = info.get("NSDownloadsFolderUsageDescription")
    if not isinstance(downloads_purpose, str) or len(downloads_purpose.strip()) < 24:
        fail("macOS Downloads export purpose text is absent or too vague")

    try:
        signer = MACOS_SIGNER.read_text(encoding="utf-8")
        desktop_main = DESKTOP_MAIN.read_text(encoding="utf-8")
    except OSError as error:
        fail(f"macOS local signing contract is unavailable: {error}")
    required_signer_fragments = (
        "FRAME_CODESIGN_IDENTITY",
        "EXPECTED_IDENTIFIER=xyz.engmanager.frame",
        "codesign --verify --deep --strict",
        "--requirements '=designated => identifier \"xyz.engmanager.frame\"'",
        "verify-trusted",
        'TeamIdentifier=',
        '-R="$test_requirement"',
    )
    if any(fragment not in signer for fragment in required_signer_fragments):
        fail("macOS local signer no longer seals and verifies the stable bundle identity")
    if 'let exports = data.join("exports");' not in desktop_main:
        fail("desktop setup no longer keeps automatic exports outside protected user folders")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dist", type=Path, default=DEFAULT_DIST)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--icons-only", action="store_true")
    args = parser.parse_args()

    tauri = json.loads(
        (ROOT / "apps" / "desktop" / "tauri.conf.json").read_text(encoding="utf-8")
    )
    validate_icon_contract(tauri)
    validate_macos_capture_contract(tauri)
    if args.icons_only:
        print(f"desktop icon contract is deterministic ({len(ICON_SIZES)} Windows sizes)")
        return 0

    dist = args.dist.resolve()
    if not dist.is_dir():
        fail(f"missing dist directory: {dist}")
    files = sorted(path for path in dist.rglob("*") if path.is_file())
    relative = [path.relative_to(dist).as_posix() for path in files]
    if any("snippets/" in path or path.endswith(".map") for path in relative):
        fail("stale snippet or source-map output is present")

    index = dist / "index.html"
    css = one(list(dist.glob("app-*.css")), "fingerprinted stylesheet")
    javascript = one(
        list(dist.glob("frame-desktop-ui-*.js")), "fingerprinted JavaScript loader"
    )
    wasm = one(
        list(dist.glob("frame-desktop-ui-*_bg.wasm")), "fingerprinted Wasm module"
    )
    expected = sorted(
        path.relative_to(dist).as_posix()
        for path in (index, css, javascript, wasm)
    )
    if relative != expected:
        fail(f"unexpected bundle file set: {relative}")
    if wasm.read_bytes()[:4] != b"\x00asm":
        fail("Wasm artifact has an invalid magic header")

    html = index.read_text(encoding="utf-8")
    if "http://" in html or "https://" in html or "tauri.js" in html:
        fail("index contains a remote or removed JavaScript dependency")
    for asset in (css, javascript, wasm):
        if f"/{asset.name}" not in html:
            fail(f"index does not reference {asset.name}")
    references = set(re.findall(r"['\"](/[A-Za-z0-9_./-]+)", html))
    missing = sorted(
        reference for reference in references if not (dist / reference[1:]).is_file()
    )
    if missing:
        fail(f"index references missing assets: {missing}")

    csp = tauri["app"]["security"]["csp"]
    if "script-src 'self' 'wasm-unsafe-eval'" not in csp:
        fail("production CSP does not permit only the Wasm compilation primitive")
    if "'unsafe-eval'" in csp.replace("'wasm-unsafe-eval'", ""):
        fail("production CSP enables general unsafe-eval")
    if tauri["app"].get("withGlobalTauri") is not True:
        fail("the supported Leptos bridge is disabled")
    if tauri["app"]["security"].get("capabilities") != ["frame-desktop-windows"]:
        fail("unexpected capability selection")

    capability = json.loads(
        (ROOT / "apps" / "desktop" / "capabilities" / "main.json").read_text(
            encoding="utf-8"
        )
    )
    if capability.get("windows") != ["main"]:
        fail("bootstrap capability is not restricted to the main window")
    expected_permissions = [
        "allow-bootstrap-main",
        "allow-bootstrap-desktop",
        "allow-dispatch-main",
        "allow-finalize-instant",
    ]
    if capability.get("permissions") != expected_permissions:
        fail("desktop capability drifted from the four-command boundary")
    explicit_permissions = (
        ROOT / "apps" / "desktop" / "permissions" / "desktop.toml"
    ).read_text(encoding="utf-8")
    for command in ("bootstrap_desktop", "dispatch_main"):
        if f'commands.allow = ["{command}"]' not in explicit_permissions:
            fail(f"desktop permission does not isolate {command}")
    finalize_permission = (
        ROOT
        / "apps"
        / "desktop"
        / "permissions"
        / "autogenerated"
        / "finalize_instant.toml"
    ).read_text(encoding="utf-8")
    for marker in (
        'identifier = "allow-finalize-instant"',
        'commands.allow = ["finalize_instant"]',
        'identifier = "deny-finalize-instant"',
        'commands.deny = ["finalize_instant"]',
    ):
        if marker not in finalize_permission:
            fail("generated finalize permission is not an isolated allow/deny pair")
    if capability.get("platforms") != ["macOS", "windows"]:
        fail("desktop platform boundary drifted")

    records = [
        {
            "path": path.relative_to(dist).as_posix(),
            "bytes": path.stat().st_size,
            "sha256": sha256(path),
        }
        for path in files
    ]
    evidence = {
        "schema_version": 1,
        "evidence_class": "deterministic_desktop_bundle",
        "platform": os.environ.get("RUNNER_OS", "local"),
        "files": records,
        "csp_sha256": hashlib.sha256(csp.encode()).hexdigest(),
        "capability_sha256": sha256(
            ROOT / "apps" / "desktop" / "capabilities" / "main.json"
        ),
        "icon_png_sha256": sha256(ICON_PNG),
        "icon_ico_sha256": sha256(ICON_ICO),
        "macos_info_plist_sha256": sha256(MACOS_INFO_PLIST),
        "icon_sizes": list(ICON_SIZES),
    }
    if args.evidence:
        output = args.evidence.resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    print(f"desktop bundle is closed and reproducible ({len(files)} files)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
