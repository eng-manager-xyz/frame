#!/usr/bin/env python3
"""Run the signed Frame application as one protected lifecycle matrix cell."""

from __future__ import annotations

import argparse
import importlib.util
import os
import pathlib
import re
import secrets
import subprocess
import sys
import tempfile
from typing import NoReturn


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR_PATH = ROOT / "scripts" / "ci" / "desktop-product-hardware.py"
VALIDATOR_SPEC = importlib.util.spec_from_file_location(
    "frame_desktop_product_hardware_validator", VALIDATOR_PATH
)
if VALIDATOR_SPEC is None or VALIDATOR_SPEC.loader is None:
    raise RuntimeError("cannot load desktop product hardware validator")
VALIDATOR = importlib.util.module_from_spec(VALIDATOR_SPEC)
sys.modules[VALIDATOR_SPEC.name] = VALIDATOR
VALIDATOR_SPEC.loader.exec_module(VALIDATOR)

DRIVER_TIMEOUT_SECONDS = 15 * 60
MAX_FAILURE_OUTPUT_BYTES = 4 * 1_024
MAX_DRIVER_EVIDENCE_BYTES = 16 * 1_024


def fail(message: str) -> NoReturn:
    raise SystemExit(f"desktop product hardware runner failed: {message}")


def prepare_output_path(path: pathlib.Path) -> pathlib.Path:
    absolute = path.absolute()
    absolute.parent.mkdir(parents=True, exist_ok=True)
    if absolute.is_symlink() or absolute.exists():
        fail("evidence output must be a new non-symlink path")
    return absolute.parent.resolve(strict=True) / absolute.name


def driver_command(
    artifact: VALIDATOR.VerifiedArtifact,
    data_root: pathlib.Path,
    *,
    source_sha: str,
    run_id: str,
    platform: str,
    topology: str,
) -> list[str]:
    return [
        str(artifact.executable),
        "--frame-product-hardware-driver",
        "--data-root",
        str(data_root),
        "--source-sha",
        source_sha,
        "--run-id",
        run_id,
        "--platform",
        platform,
        "--topology",
        topology,
        "--signing-identity",
        artifact.signing_identity,
        "--binary-sha256",
        artifact.binary_sha256,
        "--signature-binding-sha256",
        artifact.signature_binding_sha256,
        "--application-id",
        VALIDATOR.APPLICATION_ID,
    ]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--platform", choices=VALIDATOR.PLATFORMS, required=True)
    parser.add_argument("--topology", choices=VALIDATOR.TOPOLOGIES, required=True)
    parser.add_argument("--app-bundle", type=pathlib.Path)
    parser.add_argument("--binary", type=pathlib.Path)
    parser.add_argument("--expected-source-sha", required=True)
    parser.add_argument("--expected-run-id", required=True)
    parser.add_argument("--expected-signing-identity", required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()

    if os.environ.get("FRAME_REAL_HARDWARE") != "1":
        fail("FRAME_REAL_HARDWARE=1 is required on a protected runner")
    if not re.fullmatch(r"[0-9a-f]{40}", args.expected_source_sha):
        fail("expected source SHA must be an exact lowercase commit SHA")
    if not re.fullmatch(r"[A-Za-z0-9_.:-]{1,128}", args.expected_run_id):
        fail("expected run id is malformed")
    artifact = VALIDATOR.verify_artifact(
        args.platform,
        args.app_bundle,
        args.binary,
        args.expected_signing_identity,
    )
    output = prepare_output_path(args.output)

    with tempfile.TemporaryDirectory(
        prefix=".frame-product-hardware-", dir=output.parent
    ) as temporary:
        data_root = pathlib.Path(temporary).resolve(strict=True)
        environment = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("DYLD_")
        }
        environment["FRAME_REAL_HARDWARE"] = "1"
        environment["FRAME_DESKTOP_FAKE_PIPELINE"] = "0"
        environment["FRAME_HARDWARE_DRIVER_TOKEN"] = secrets.token_hex(32)
        command = driver_command(
            artifact,
            data_root,
            source_sha=args.expected_source_sha,
            run_id=args.expected_run_id,
            platform=args.platform,
            topology=args.topology,
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
            fail("signed Frame product hardware driver exceeded fifteen minutes")
        if result.returncode != 0:
            detail = f"{result.stdout}{result.stderr}".encode(
                "utf-8", errors="replace"
            )[:MAX_FAILURE_OUTPUT_BYTES].decode("utf-8", errors="replace")
            fail(
                f"signed Frame product hardware driver exited {result.returncode}: "
                f"{detail.strip()}"
            )
        driver_evidence = data_root / "evidence.json"
        try:
            metadata = driver_evidence.lstat()
        except OSError as error:
            fail(f"signed Frame product driver omitted evidence: {error}")
        if (
            driver_evidence.is_symlink()
            or not driver_evidence.is_file()
            or metadata.st_size == 0
            or metadata.st_size > MAX_DRIVER_EVIDENCE_BYTES
        ):
            fail("signed Frame product driver emitted invalid evidence")
        os.replace(driver_evidence, output)
    print(
        f"signed Frame {args.platform} {args.topology} product hardware driver completed"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
