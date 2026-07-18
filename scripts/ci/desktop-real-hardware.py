#!/usr/bin/env python3
"""Validate narrow macOS display evidence from the protected hardware driver."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import plistlib
import re
import stat
import subprocess
import sys
from pathlib import Path
from typing import NoReturn


MACOS_DISPLAY_CAPABILITY = "macos_display_webm_v1"
MACOS_DISPLAY_ADAPTER = "native_macos_display"
MACOS_BUNDLE_IDENTIFIER = "xyz.engmanager.frame"
MACOS_DISPLAY_CASES = (
    "screen_capture_preauthorized",
    "display_catalog_and_selection",
    "display_capture",
    "frame_window_exclusion",
    "stop_and_playable_webm",
    "export_and_playable_webm",
    "cancel_partial_cleanup",
)

# These remain protected release requirements. The current display-only driver
# cannot claim them, and this validator deliberately has no full-product mode.
PROTECTED_FULL_PRODUCT_CASES = (
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


class DuplicateEvidenceKey(ValueError):
    """Raised when evidence contains an ambiguous duplicate JSON key."""


def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise DuplicateEvidenceKey(key)
        result[key] = value
    return result


def command_output(*command: str) -> str:
    result = subprocess.run(command, check=False, capture_output=True, text=True)
    output = f"{result.stdout}{result.stderr}"
    if result.returncode != 0:
        fail(f"{' '.join(command[:2])} failed: {output.strip()}")
    return output


def metadata_value(details: str, key: str) -> str:
    match = re.search(rf"^{re.escape(key)}=(.+)$", details, re.MULTILINE)
    if match is None:
        fail(f"code signature does not report {key}")
    return match.group(1).strip()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_signed_bundle(
    app_bundle: Path, expected_team: str
) -> tuple[str, str, str]:
    if sys.platform != "darwin":
        fail("signed application validation requires macOS")
    if app_bundle.is_symlink() or not app_bundle.is_dir():
        fail("app bundle must be a non-symlink directory")
    plist_path = app_bundle / "Contents" / "Info.plist"
    try:
        with plist_path.open("rb") as source:
            plist = plistlib.load(source)
    except (OSError, plistlib.InvalidFileException) as error:
        fail(f"application Info.plist is unavailable: {error}")
    if not isinstance(plist, dict):
        fail("application Info.plist root must be a dictionary")
    if plist.get("CFBundleIdentifier") != MACOS_BUNDLE_IDENTIFIER:
        fail(f"application bundle id must be {MACOS_BUNDLE_IDENTIFIER}")
    executable_name = plist.get("CFBundleExecutable")
    if not isinstance(executable_name, str) or not re.fullmatch(
        r"[A-Za-z0-9._-]{1,128}", executable_name
    ):
        fail("CFBundleExecutable is missing or unsafe")
    executable = app_bundle / "Contents" / "MacOS" / executable_name
    try:
        executable_stat = executable.lstat()
    except OSError as error:
        fail(f"bundle executable is unavailable: {error}")
    if (
        executable.is_symlink()
        or not stat.S_ISREG(executable_stat.st_mode)
        or executable_stat.st_mode & 0o111 == 0
    ):
        fail("bundle executable must be an executable non-symlink regular file")

    command_output(
        "codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app_bundle)
    )
    details = command_output("codesign", "--display", "--verbose=4", str(app_bundle))
    if metadata_value(details, "Identifier") != MACOS_BUNDLE_IDENTIFIER:
        fail("code-signing identifier does not match the application bundle id")
    if "Signature=adhoc" in details:
        fail("ad-hoc signatures cannot satisfy protected ScreenCaptureKit evidence")
    team = metadata_value(details, "TeamIdentifier")
    if team != expected_team:
        fail("code-signing team does not match the protected expected team")
    test_requirement = (
        f'anchor apple generic and identifier "{MACOS_BUNDLE_IDENTIFIER}" '
        f'and certificate leaf[subject.OU] = "{expected_team}"'
    )
    command_output(
        "codesign",
        "--verify",
        "--deep",
        "--strict",
        "--verbose=2",
        f"-R={test_requirement}",
        str(app_bundle),
    )
    requirement_output = command_output(
        "codesign", "--display", "--requirements", "-", str(app_bundle)
    )
    requirement_match = re.search(
        r"^designated => .+$", requirement_output, re.MULTILINE
    )
    if requirement_match is None:
        fail("signed application has no designated requirement")
    requirement = requirement_match.group(0)
    stable_prefix = (
        f'designated => identifier "{MACOS_BUNDLE_IDENTIFIER}" '
        "and anchor apple generic"
    )
    team_clause = f'certificate leaf[subject.OU] = "{expected_team}"'
    if not requirement.startswith(stable_prefix) or team_clause not in requirement:
        fail("designated requirement is not certificate-backed and bundle-bound")

    executable_sha256 = sha256_file(executable)
    requirement_sha256 = hashlib.sha256(requirement.encode("utf-8")).hexdigest()
    return executable_sha256, team, requirement_sha256


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--app-bundle", type=Path, required=True)
    parser.add_argument("--expected-source-sha", required=True)
    parser.add_argument("--expected-run-id", required=True)
    parser.add_argument("--expected-signing-team", required=True)
    parser.add_argument(
        "--expected-capability",
        choices=(MACOS_DISPLAY_CAPABILITY,),
        required=True,
    )
    parser.add_argument("--require-hardware", action="store_true")
    args = parser.parse_args()

    if args.require_hardware and os.environ.get("FRAME_REAL_HARDWARE") != "1":
        fail("FRAME_REAL_HARDWARE=1 is required on a protected runner")
    if not re.fullmatch(r"[0-9a-f]{40}", args.expected_source_sha):
        fail("expected source SHA must be an exact lowercase commit SHA")
    if not re.fullmatch(r"[A-Za-z0-9_.:-]{1,128}", args.expected_run_id):
        fail("expected run id is malformed")
    if not re.fullmatch(r"[A-Z0-9]{10}", args.expected_signing_team):
        fail("expected Apple signing team must be a ten-character team id")
    executable_sha256, signing_team, requirement_sha256 = verify_signed_bundle(
        args.app_bundle, args.expected_signing_team
    )
    if not args.evidence.is_file():
        fail(f"missing evidence file {args.evidence}")
    raw = args.evidence.read_bytes()
    try:
        evidence = json.loads(raw, object_pairs_hook=reject_duplicate_keys)
    except json.JSONDecodeError as error:
        fail(f"invalid JSON: {error.msg}")
    except DuplicateEvidenceKey as error:
        fail(f"duplicate JSON key: {error}")
    if not isinstance(evidence, dict):
        fail("evidence must be a JSON object")

    schema_version = evidence.get("schema_version")
    if type(schema_version) is not int or schema_version != 1:
        fail("unsupported schema_version")
    if evidence.get("evidence_class") != "macos_display_capture_partial":
        fail("evidence_class must identify the partial macOS display gate")
    if evidence.get("full_product_gate") != "not_claimed":
        fail("display evidence must not claim the protected full-product gate")
    if evidence.get("capability") != args.expected_capability:
        fail("capability does not match the independently expected capability")
    if evidence.get("platform") != "macos":
        fail("only macOS is accepted by the display-capture hardware gate")
    if evidence.get("adapter") != MACOS_DISPLAY_ADAPTER:
        fail(f"adapter must be {MACOS_DISPLAY_ADAPTER}")
    if evidence.get("source_sha") != args.expected_source_sha:
        fail("evidence source SHA does not match the checked-out candidate")
    if evidence.get("run_id") != args.expected_run_id:
        fail("evidence run id does not match this protected workflow run")
    if evidence.get("bundle_identifier") != MACOS_BUNDLE_IDENTIFIER:
        fail("evidence bundle identifier does not match Frame")
    if evidence.get("signing_team_id") != signing_team:
        fail("evidence signing team does not match the verified bundle")
    if evidence.get("binary_sha256") != executable_sha256:
        fail("evidence binary digest does not match the signed bundle executable")
    if evidence.get("designated_requirement_sha256") != requirement_sha256:
        fail("evidence designated requirement does not match the signed bundle")
    cases = evidence.get("cases")
    if not isinstance(cases, dict):
        fail("cases must be an object")
    expected_cases = set(MACOS_DISPLAY_CASES)
    actual_cases = set(cases)
    if actual_cases != expected_cases:
        fail(
            "case set differs from the exact macOS display capability: "
            f"missing={sorted(expected_cases - actual_cases)}, "
            f"unexpected={sorted(actual_cases - expected_cases)}"
        )
    failed = [case for case in MACOS_DISPLAY_CASES if cases.get(case) is not True]
    if failed:
        fail(f"required cases did not pass: {failed}")

    print(
        "macOS display hardware evidence passed without claiming the "
        f"{len(PROTECTED_FULL_PRODUCT_CASES)}-case full-product gate "
        f"(sha256={hashlib.sha256(raw).hexdigest()})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
