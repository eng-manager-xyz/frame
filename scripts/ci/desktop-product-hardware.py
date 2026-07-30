#!/usr/bin/env python3
"""Validate signed macOS/Windows desktop lifecycle hardware evidence."""

from __future__ import annotations

import argparse
import base64
import hashlib
import importlib.util
import json
import os
import pathlib
import plistlib
import re
import stat
import subprocess
import sys
from dataclasses import dataclass
from typing import NoReturn


ROOT = pathlib.Path(__file__).resolve().parents[2]
PARTIAL_VALIDATOR_PATH = ROOT / "scripts" / "ci" / "desktop-real-hardware.py"
PARTIAL_VALIDATOR_SPEC = importlib.util.spec_from_file_location(
    "frame_partial_desktop_hardware_validator", PARTIAL_VALIDATOR_PATH
)
if PARTIAL_VALIDATOR_SPEC is None or PARTIAL_VALIDATOR_SPEC.loader is None:
    raise RuntimeError("cannot load partial desktop hardware validator")
PARTIAL_VALIDATOR = importlib.util.module_from_spec(PARTIAL_VALIDATOR_SPEC)
sys.modules[PARTIAL_VALIDATOR_SPEC.name] = PARTIAL_VALIDATOR
PARTIAL_VALIDATOR_SPEC.loader.exec_module(PARTIAL_VALIDATOR)

CAPABILITY = "desktop_lifecycle_matrix_v1"
EVIDENCE_CLASS = "desktop_product_hardware_matrix_cell"
APPLICATION_ID = "xyz.engmanager.frame"
PLATFORMS = ("macos", "windows")
TOPOLOGIES = ("single-standard", "dual-mixed-scale", "rotated")
ADAPTERS = {
    "macos": "native_macos_display",
    "windows": "native_windows_display_window_region",
}
CASES = (
    "signed_native_application",
    "native_capture_adapter",
    "three_content_protected_windows",
    "global_hotkey_registration_and_handler",
    "tray_registration_and_handler",
    "overlay_target_picker_close_reopen",
    "monitor_relative_window_placement",
    "randomized_physical_window_exclusion",
)
MEASUREMENTS = (
    "monitor_count",
    "distinct_scale_count",
    "rotated_display_count",
)
MAX_EVIDENCE_BYTES = 16 * 1_024


def fail(message: str) -> NoReturn:
    raise SystemExit(f"desktop product hardware evidence failed: {message}")


class DuplicateEvidenceKey(ValueError):
    """Raised when evidence contains an ambiguous duplicate JSON key."""


def reject_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise DuplicateEvidenceKey(key)
        result[key] = value
    return result


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_file(path: pathlib.Path, suffix: str) -> pathlib.Path:
    absolute = path.absolute()
    if absolute.is_symlink():
        fail("signed executable path must not be a symlink")
    try:
        canonical = absolute.resolve(strict=True)
        metadata = canonical.lstat()
    except OSError as error:
        fail(f"signed executable is unavailable: {error}")
    if (
        canonical.suffix.lower() != suffix
        or canonical.is_symlink()
        or not stat.S_ISREG(metadata.st_mode)
    ):
        fail("signed executable must be a non-symlink regular file")
    return canonical


def macos_bundle_executable(app_bundle: pathlib.Path) -> pathlib.Path:
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
    executable = (app_bundle / "Contents" / "MacOS" / name).absolute()
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
    return executable.resolve(strict=True)


@dataclass(frozen=True)
class VerifiedArtifact:
    executable: pathlib.Path
    binary_sha256: str
    signing_identity: str
    signature_binding_sha256: str


def verify_macos_artifact(
    app_bundle: pathlib.Path, expected_team: str
) -> VerifiedArtifact:
    binary_sha256, signing_team, requirement_sha256 = (
        PARTIAL_VALIDATOR.verify_signed_bundle(app_bundle, expected_team)
    )
    return VerifiedArtifact(
        executable=macos_bundle_executable(app_bundle),
        binary_sha256=binary_sha256,
        signing_identity=signing_team,
        signature_binding_sha256=requirement_sha256,
    )


def powershell_output(script: str, argument: pathlib.Path) -> str:
    result = subprocess.run(
        [
            "powershell.exe",
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
            str(argument),
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=60,
    )
    output = f"{result.stdout}{result.stderr}".strip()
    if result.returncode != 0:
        fail(f"Windows signature verification failed: {output[:1024]}")
    return output


def verify_windows_artifact(
    binary: pathlib.Path, expected_thumbprint: str
) -> VerifiedArtifact:
    if sys.platform != "win32":
        fail("Windows signed executable validation requires Windows")
    if not re.fullmatch(r"[A-F0-9]{40}", expected_thumbprint):
        fail("expected Windows certificate thumbprint is malformed")
    executable = canonical_file(binary, ".exe")
    signature_script = (
        "$s=Get-AuthenticodeSignature -LiteralPath $args[0];"
        "if($s.Status -ne 'Valid' -or $null -eq $s.SignerCertificate){exit 1};"
        "$c=$s.SignerCertificate;"
        "[pscustomobject]@{status=[string]$s.Status;"
        "thumbprint=[string]$c.Thumbprint;"
        "certificate=[Convert]::ToBase64String($c.RawData)}"
        "|ConvertTo-Json -Compress"
    )
    raw = powershell_output(signature_script, executable)
    try:
        details = json.loads(raw, object_pairs_hook=reject_duplicate_keys)
    except (json.JSONDecodeError, DuplicateEvidenceKey) as error:
        fail(f"Windows signature metadata is invalid: {error}")
    if (
        not isinstance(details, dict)
        or set(details) != {"status", "thumbprint", "certificate"}
        or details.get("status") != "Valid"
        or details.get("thumbprint") != expected_thumbprint
        or not isinstance(details.get("certificate"), str)
    ):
        fail("Windows signature metadata does not match the protected certificate")
    try:
        certificate = base64.b64decode(
            details["certificate"], validate=True  # type: ignore[arg-type]
        )
    except (ValueError, TypeError) as error:
        fail(f"Windows signing certificate is malformed: {error}")
    if not certificate or len(certificate) > 64 * 1_024:
        fail("Windows signing certificate is outside the size bound")
    return VerifiedArtifact(
        executable=executable,
        binary_sha256=sha256_file(executable),
        signing_identity=expected_thumbprint,
        signature_binding_sha256=hashlib.sha256(certificate).hexdigest(),
    )


def verify_artifact(
    platform: str,
    app_bundle: pathlib.Path | None,
    binary: pathlib.Path | None,
    expected_signing_identity: str,
) -> VerifiedArtifact:
    if platform == "macos":
        if app_bundle is None or binary is not None:
            fail("macOS validation requires only --app-bundle")
        return verify_macos_artifact(app_bundle, expected_signing_identity)
    if platform == "windows":
        if binary is None or app_bundle is not None:
            fail("Windows validation requires only --binary")
        return verify_windows_artifact(binary, expected_signing_identity)
    fail("unsupported hardware platform")


def load_evidence(path: pathlib.Path) -> dict[str, object]:
    try:
        metadata = path.lstat()
    except OSError as error:
        fail(f"evidence is unavailable: {error}")
    if (
        path.is_symlink()
        or not stat.S_ISREG(metadata.st_mode)
        or metadata.st_size == 0
        or metadata.st_size > MAX_EVIDENCE_BYTES
    ):
        fail("evidence must be a bounded non-symlink regular file")
    try:
        evidence = json.loads(
            path.read_bytes(), object_pairs_hook=reject_duplicate_keys
        )
    except json.JSONDecodeError as error:
        fail(f"invalid JSON: {error.msg}")
    except DuplicateEvidenceKey as error:
        fail(f"duplicate JSON key: {error}")
    if not isinstance(evidence, dict):
        fail("evidence must be a JSON object")
    return evidence


def validate_measurements(topology: str, value: object) -> None:
    if not isinstance(value, dict) or set(value) != set(MEASUREMENTS):
        fail("measurements must contain the exact coarse topology fields")
    if any(
        type(value.get(field)) is not int  # noqa: E721 - bool must be rejected
        for field in MEASUREMENTS
    ):
        fail("topology measurements must be exact integers")
    monitor_count = value["monitor_count"]
    scale_count = value["distinct_scale_count"]
    rotated_count = value["rotated_display_count"]
    assert isinstance(monitor_count, int)
    assert isinstance(scale_count, int)
    assert isinstance(rotated_count, int)
    if not 1 <= monitor_count <= 2 or not 1 <= scale_count <= monitor_count:
        fail("monitor or scale counts are outside the protected bounds")
    if not 0 <= rotated_count <= monitor_count:
        fail("rotated display count is outside the protected bounds")
    valid = (
        topology == "single-standard"
        and (monitor_count, scale_count, rotated_count) == (1, 1, 0)
    ) or (
        topology == "dual-mixed-scale"
        and (monitor_count, scale_count, rotated_count) == (2, 2, 0)
    ) or (
        topology == "rotated"
        and monitor_count in (1, 2)
        and rotated_count >= 1
    )
    if not valid:
        fail("coarse measurements do not match the requested topology cell")


def validate_evidence(
    evidence: dict[str, object],
    *,
    platform: str,
    topology: str,
    source_sha: str,
    run_id: str,
    artifact: VerifiedArtifact,
) -> None:
    expected_keys = {
        "schema_version",
        "evidence_class",
        "capability",
        "platform",
        "topology",
        "adapter",
        "source_sha",
        "run_id",
        "application_id",
        "signing_identity",
        "binary_sha256",
        "signature_binding_sha256",
        "cases",
        "measurements",
    }
    if set(evidence) != expected_keys:
        fail("evidence contains missing or unapproved top-level fields")
    expected = {
        "schema_version": 1,
        "evidence_class": EVIDENCE_CLASS,
        "capability": CAPABILITY,
        "platform": platform,
        "topology": topology,
        "adapter": ADAPTERS[platform],
        "source_sha": source_sha,
        "run_id": run_id,
        "application_id": APPLICATION_ID,
        "signing_identity": artifact.signing_identity,
        "binary_sha256": artifact.binary_sha256,
        "signature_binding_sha256": artifact.signature_binding_sha256,
    }
    for field, value in expected.items():
        if evidence.get(field) != value:
            fail(f"evidence {field} does not match the independently expected value")
    if type(evidence.get("schema_version")) is not int:  # noqa: E721
        fail("schema_version must be an exact integer")
    cases = evidence.get("cases")
    if not isinstance(cases, dict) or set(cases) != set(CASES):
        fail("cases must contain the exact lifecycle hardware case set")
    for name in CASES:
        if type(cases.get(name)) is not bool or cases[name] is not True:  # noqa: E721
            fail(f"case {name} did not pass as a strict boolean")
    validate_measurements(topology, evidence.get("measurements"))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path, required=True)
    parser.add_argument("--platform", choices=PLATFORMS, required=True)
    parser.add_argument("--topology", choices=TOPOLOGIES, required=True)
    parser.add_argument("--app-bundle", type=pathlib.Path)
    parser.add_argument("--binary", type=pathlib.Path)
    parser.add_argument("--expected-source-sha", required=True)
    parser.add_argument("--expected-run-id", required=True)
    parser.add_argument("--expected-signing-identity", required=True)
    parser.add_argument("--require-hardware", action="store_true")
    args = parser.parse_args()

    if args.require_hardware and os.environ.get("FRAME_REAL_HARDWARE") != "1":
        fail("FRAME_REAL_HARDWARE=1 is required on a protected runner")
    if not re.fullmatch(r"[0-9a-f]{40}", args.expected_source_sha):
        fail("expected source SHA must be an exact lowercase commit SHA")
    if not re.fullmatch(r"[A-Za-z0-9_.:-]{1,128}", args.expected_run_id):
        fail("expected run id is malformed")
    artifact = verify_artifact(
        args.platform,
        args.app_bundle,
        args.binary,
        args.expected_signing_identity,
    )
    evidence = load_evidence(args.evidence)
    validate_evidence(
        evidence,
        platform=args.platform,
        topology=args.topology,
        source_sha=args.expected_source_sha,
        run_id=args.expected_run_id,
        artifact=artifact,
    )
    print(
        f"signed Frame {args.platform} {args.topology} lifecycle hardware evidence passed"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
