#!/usr/bin/env python3
"""Run the signed Frame binary as the protected macOS display driver."""

from __future__ import annotations

import argparse
import importlib.util
import os
import pathlib
import plistlib
import re
import secrets
import stat
import subprocess
import sys
import tempfile
from typing import NoReturn


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR_PATH = ROOT / "scripts" / "ci" / "desktop-real-hardware.py"
VALIDATOR_SPEC = importlib.util.spec_from_file_location(
    "frame_desktop_real_hardware_validator", VALIDATOR_PATH
)
if VALIDATOR_SPEC is None or VALIDATOR_SPEC.loader is None:
    raise RuntimeError("cannot load desktop real-hardware validator")
VALIDATOR = importlib.util.module_from_spec(VALIDATOR_SPEC)
sys.modules[VALIDATOR_SPEC.name] = VALIDATOR
VALIDATOR_SPEC.loader.exec_module(VALIDATOR)

DRIVER_TIMEOUT_SECONDS = 10 * 60
MAX_FAILURE_OUTPUT_BYTES = 4 * 1_024


def fail(message: str) -> NoReturn:
    raise SystemExit(f"desktop real-hardware runner failed: {message}")


def bundle_executable(app_bundle: pathlib.Path) -> pathlib.Path:
    plist_path = app_bundle / "Contents" / "Info.plist"
    try:
        with plist_path.open("rb") as source:
            plist = plistlib.load(source)
    except (OSError, plistlib.InvalidFileException) as error:
        fail(f"application Info.plist is unavailable: {error}")
    if not isinstance(plist, dict):
        fail("application Info.plist root must be a dictionary")
    name = plist.get("CFBundleExecutable")
    if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9._-]{1,128}", name):
        fail("CFBundleExecutable is missing or unsafe")
    executable = app_bundle / "Contents" / "MacOS" / name
    try:
        metadata = executable.lstat()
    except OSError as error:
        fail(f"bundle executable is unavailable: {error}")
    if (
        executable.is_symlink()
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_mode & 0o111 == 0
    ):
        fail("bundle executable must be an executable non-symlink regular file")
    return executable


def canonical_bundle_path(path: pathlib.Path) -> pathlib.Path:
    absolute = path.absolute()
    if absolute.is_symlink():
        fail("application bundle path must not be a symlink")
    try:
        canonical = absolute.resolve(strict=True)
    except OSError as error:
        fail(f"application bundle is unavailable: {error}")
    return canonical


def prepare_output_path(path: pathlib.Path) -> pathlib.Path:
    absolute = path.absolute()
    absolute.parent.mkdir(parents=True, exist_ok=True)
    if absolute.is_symlink() or absolute.exists():
        fail("evidence output must be a new non-symlink path")
    return absolute.parent.resolve(strict=True) / absolute.name


def driver_command(
    executable: pathlib.Path,
    data_root: pathlib.Path,
    source_sha: str,
    run_id: str,
    signing_team: str,
    binary_sha256: str,
    designated_requirement_sha256: str,
) -> list[str]:
    return [
        str(executable),
        "--frame-hardware-driver",
        "--data-root",
        str(data_root),
        "--source-sha",
        source_sha,
        "--run-id",
        run_id,
        "--signing-team",
        signing_team,
        "--binary-sha256",
        binary_sha256,
        "--designated-requirement-sha256",
        designated_requirement_sha256,
        "--bundle-identifier",
        VALIDATOR.MACOS_BUNDLE_IDENTIFIER,
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app-bundle", type=pathlib.Path, required=True)
    parser.add_argument("--expected-source-sha", required=True)
    parser.add_argument("--expected-run-id", required=True)
    parser.add_argument("--expected-signing-team", required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()

    if os.environ.get("FRAME_REAL_HARDWARE") != "1":
        fail("FRAME_REAL_HARDWARE=1 is required on a protected runner")
    if not re.fullmatch(r"[0-9a-f]{40}", args.expected_source_sha):
        fail("expected source SHA must be an exact lowercase commit SHA")
    if not re.fullmatch(r"[A-Za-z0-9_.:-]{1,128}", args.expected_run_id):
        fail("expected run id is malformed")
    if not re.fullmatch(r"[A-Z0-9]{10}", args.expected_signing_team):
        fail("expected Apple signing team must be a ten-character team id")

    app_bundle = canonical_bundle_path(args.app_bundle)
    binary_sha256, signing_team, requirement_sha256 = VALIDATOR.verify_signed_bundle(
        app_bundle, args.expected_signing_team
    )
    executable = bundle_executable(app_bundle)
    output = prepare_output_path(args.output)

    with tempfile.TemporaryDirectory(
        prefix=".frame-hardware-", dir=output.parent
    ) as temporary:
        data_root = pathlib.Path(temporary).resolve()
        environment = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("DYLD_")
        }
        environment["FRAME_REAL_HARDWARE"] = "1"
        environment["FRAME_DESKTOP_FAKE_PIPELINE"] = "0"
        environment["FRAME_HARDWARE_DRIVER_TOKEN"] = secrets.token_hex(32)
        command = driver_command(
            executable,
            data_root,
            args.expected_source_sha,
            args.expected_run_id,
            signing_team,
            binary_sha256,
            requirement_sha256,
        )
        try:
            result = subprocess.run(
                command,
                cwd=ROOT,
                env=environment,
                check=False,
                capture_output=True,
                text=True,
                timeout=DRIVER_TIMEOUT_SECONDS,
            )
        except subprocess.TimeoutExpired:
            fail("signed Frame hardware driver exceeded ten minutes")
        if result.returncode != 0:
            detail = f"{result.stdout}{result.stderr}".encode(
                "utf-8", errors="replace"
            )[:MAX_FAILURE_OUTPUT_BYTES].decode("utf-8", errors="replace")
            fail(
                f"signed Frame hardware driver exited {result.returncode}: "
                f"{detail.strip()}"
            )
        driver_evidence = data_root / "evidence.json"
        if (
            driver_evidence.is_symlink()
            or not driver_evidence.is_file()
            or driver_evidence.stat().st_size > 16 * 1_024
        ):
            fail("signed Frame hardware driver omitted bounded evidence")
        os.replace(driver_evidence, output)
    print("signed Frame macOS display hardware driver completed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
