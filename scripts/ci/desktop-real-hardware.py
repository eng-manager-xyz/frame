#!/usr/bin/env python3
"""Validate evidence emitted by the protected desktop hardware driver."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
from pathlib import Path
from typing import NoReturn


REQUIRED_CASES = (
    "permission_denied_and_recovery",
    "display_window_region_capture",
    "frame_window_exclusion",
    "multi_monitor_scale_rotation_placement",
    "microphone_system_audio_camera",
    "device_loss_and_hotplug",
    "sleep_wake_recovery",
    "instant_and_studio",
    "pause_resume_stop_cancel",
    "tray_hotkey_overlay_lifecycle",
    "crash_restart_recovery",
    "update_relaunch",
    "keyboard_only_journey",
    "screen_reader_journey",
)


def fail(message: str) -> NoReturn:
    raise SystemExit(f"desktop real-hardware evidence failed: {message}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--require-hardware", action="store_true")
    args = parser.parse_args()

    if args.require_hardware and os.environ.get("FRAME_REAL_HARDWARE") != "1":
        fail("FRAME_REAL_HARDWARE=1 is required on a protected runner")
    if not args.evidence.is_file():
        fail(f"missing evidence file {args.evidence}")
    raw = args.evidence.read_bytes()
    try:
        evidence = json.loads(raw)
    except json.JSONDecodeError as error:
        fail(f"invalid JSON: {error.msg}")

    if evidence.get("schema_version") != 1:
        fail("unsupported schema_version")
    if evidence.get("adapter") == "deterministic_fake":
        fail("fake adapter cannot satisfy the hardware gate")
    if evidence.get("platform") not in {"macos", "windows"}:
        fail("platform must be macos or windows")
    if not re.fullmatch(r"[0-9a-f]{64}", str(evidence.get("binary_sha256", ""))):
        fail("binary_sha256 is missing or malformed")
    if not re.fullmatch(r"[A-Za-z0-9_.:-]{1,128}", str(evidence.get("run_id", ""))):
        fail("run_id is missing or malformed")
    cases = evidence.get("cases")
    if not isinstance(cases, dict):
        fail("cases must be an object")
    failed = [case for case in REQUIRED_CASES if cases.get(case) is not True]
    if failed:
        fail(f"required cases did not pass: {failed}")
    monitors = evidence.get("monitor_topologies")
    if not isinstance(monitors, list) or len(monitors) < 2:
        fail("at least two distinct monitor topology traces are required")
    assistive_technology = evidence.get("assistive_technology")
    if not isinstance(assistive_technology, str) or not assistive_technology.strip():
        fail("named assistive technology is required")

    print(
        "desktop real-hardware evidence passed "
        f"({evidence['platform']}, sha256={hashlib.sha256(raw).hexdigest()})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
