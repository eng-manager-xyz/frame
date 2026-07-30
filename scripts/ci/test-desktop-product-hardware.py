#!/usr/bin/env python3
"""Fail-closed tests for the signed desktop lifecycle hardware matrix."""

from __future__ import annotations

import base64
import copy
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[2]
VALIDATOR_PATH = ROOT / "scripts" / "ci" / "desktop-product-hardware.py"
RUNNER_PATH = ROOT / "scripts" / "ci" / "run-desktop-product-hardware.py"


def load_module(name: str, path: pathlib.Path) -> object:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path.name}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


VALIDATOR = load_module("frame_test_product_hardware_validator", VALIDATOR_PATH)
RUNNER = load_module("frame_test_product_hardware_runner", RUNNER_PATH)

SOURCE_SHA = "a" * 40
RUN_ID = "12345:2"
BINARY_SHA = "b" * 64
SIGNATURE_SHA = "c" * 64
MAC_TEAM = "ABCDE12345"
WINDOWS_THUMBPRINT = "A" * 40


def artifact(
    *,
    executable: pathlib.Path = pathlib.Path("/signed/frame-desktop"),
    identity: str = MAC_TEAM,
) -> object:
    return VALIDATOR.VerifiedArtifact(
        executable=executable,
        binary_sha256=BINARY_SHA,
        signing_identity=identity,
        signature_binding_sha256=SIGNATURE_SHA,
    )


def valid_evidence(
    *,
    platform: str = "macos",
    topology: str = "single-standard",
    identity: str = MAC_TEAM,
) -> dict[str, object]:
    measurements = {
        "monitor_count": 1,
        "distinct_scale_count": 1,
        "rotated_display_count": 0,
    }
    if topology == "dual-mixed-scale":
        measurements = {
            "monitor_count": 2,
            "distinct_scale_count": 2,
            "rotated_display_count": 0,
        }
    elif topology == "rotated":
        measurements = {
            "monitor_count": 2,
            "distinct_scale_count": 1,
            "rotated_display_count": 1,
        }
    return {
        "schema_version": 1,
        "evidence_class": VALIDATOR.EVIDENCE_CLASS,
        "capability": VALIDATOR.CAPABILITY,
        "platform": platform,
        "topology": topology,
        "adapter": VALIDATOR.ADAPTERS[platform],
        "source_sha": SOURCE_SHA,
        "run_id": RUN_ID,
        "application_id": VALIDATOR.APPLICATION_ID,
        "signing_identity": identity,
        "binary_sha256": BINARY_SHA,
        "signature_binding_sha256": SIGNATURE_SHA,
        "cases": {name: True for name in VALIDATOR.CASES},
        "measurements": measurements,
    }


class ProductHardwareValidatorTests(unittest.TestCase):
    def validate(
        self,
        evidence: dict[str, object],
        *,
        platform: str = "macos",
        topology: str = "single-standard",
        identity: str = MAC_TEAM,
    ) -> None:
        VALIDATOR.validate_evidence(
            evidence,
            platform=platform,
            topology=topology,
            source_sha=SOURCE_SHA,
            run_id=RUN_ID,
            artifact=artifact(identity=identity),
        )

    def test_all_platform_topology_cells_validate(self) -> None:
        for platform, identity in (
            ("macos", MAC_TEAM),
            ("windows", WINDOWS_THUMBPRINT),
        ):
            for topology in VALIDATOR.TOPOLOGIES:
                with self.subTest(platform=platform, topology=topology):
                    self.validate(
                        valid_evidence(
                            platform=platform,
                            topology=topology,
                            identity=identity,
                        ),
                        platform=platform,
                        topology=topology,
                        identity=identity,
                    )

    def test_identity_source_run_and_binary_bindings_are_exact(self) -> None:
        for field, value in (
            ("source_sha", "d" * 40),
            ("run_id", "another:run"),
            ("signing_identity", "ZZZZZ99999"),
            ("binary_sha256", "d" * 64),
            ("signature_binding_sha256", "e" * 64),
            ("application_id", "attacker.example"),
            ("adapter", "deterministic_fake"),
        ):
            with self.subTest(field=field):
                evidence = valid_evidence()
                evidence[field] = value
                with self.assertRaises(SystemExit):
                    self.validate(evidence)

    def test_case_set_and_strict_booleans_are_fail_closed(self) -> None:
        evidence = valid_evidence()
        cases = evidence["cases"]
        assert isinstance(cases, dict)
        cases.pop(VALIDATOR.CASES[0])
        with self.assertRaises(SystemExit):
            self.validate(evidence)

        evidence = valid_evidence()
        cases = evidence["cases"]
        assert isinstance(cases, dict)
        cases["unapproved_case"] = True
        with self.assertRaises(SystemExit):
            self.validate(evidence)

        for value in (False, 1, "true"):
            evidence = valid_evidence()
            cases = evidence["cases"]
            assert isinstance(cases, dict)
            cases[VALIDATOR.CASES[0]] = value
            with self.subTest(value=value), self.assertRaises(SystemExit):
                self.validate(evidence)

    def test_topology_measurements_cannot_be_faked_or_expanded(self) -> None:
        mutations = (
            {"monitor_count": 2, "distinct_scale_count": 1, "rotated_display_count": 0},
            {"monitor_count": True, "distinct_scale_count": 1, "rotated_display_count": 0},
            {"monitor_count": 1, "distinct_scale_count": 1, "rotated_display_count": 2},
            {
                "monitor_count": 1,
                "distinct_scale_count": 1,
                "rotated_display_count": 0,
                "device_name": "private",
            },
        )
        for measurements in mutations:
            evidence = valid_evidence()
            evidence["measurements"] = measurements
            with self.subTest(measurements=measurements), self.assertRaises(SystemExit):
                self.validate(evidence)

    def test_unknown_top_level_fields_are_rejected(self) -> None:
        evidence = valid_evidence()
        evidence["device_identifiers"] = ["private"]
        with self.assertRaises(SystemExit):
            self.validate(evidence)

    def test_duplicate_json_keys_and_oversized_evidence_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="frame-product-hardware-test-") as temporary:
            root = pathlib.Path(temporary)
            duplicate = root / "duplicate.json"
            duplicate.write_text(
                '{"schema_version":1,"schema_version":1}', encoding="utf-8"
            )
            with self.assertRaises(SystemExit):
                VALIDATOR.load_evidence(duplicate)
            oversized = root / "oversized.json"
            oversized.write_bytes(b"{" + b" " * VALIDATOR.MAX_EVIDENCE_BYTES + b"}")
            with self.assertRaises(SystemExit):
                VALIDATOR.load_evidence(oversized)

    def test_windows_authenticode_is_bound_to_thumbprint_and_certificate(self) -> None:
        with tempfile.TemporaryDirectory(prefix="frame-windows-signature-test-") as temporary:
            binary = pathlib.Path(temporary) / "frame-desktop.exe"
            binary.write_bytes(b"signed-frame")
            certificate = b"bounded certificate bytes"
            response = json.dumps(
                {
                    "status": "Valid",
                    "thumbprint": WINDOWS_THUMBPRINT,
                    "certificate": base64.b64encode(certificate).decode("ascii"),
                }
            )
            completed = subprocess.CompletedProcess(
                args=["powershell.exe"], returncode=0, stdout=response, stderr=""
            )
            with (
                mock.patch.object(VALIDATOR.sys, "platform", "win32"),
                mock.patch.object(VALIDATOR.subprocess, "run", return_value=completed),
            ):
                observed = VALIDATOR.verify_windows_artifact(
                    binary, WINDOWS_THUMBPRINT
                )
            self.assertEqual(observed.signing_identity, WINDOWS_THUMBPRINT)
            self.assertEqual(
                observed.binary_sha256, VALIDATOR.sha256_file(binary.resolve())
            )
            self.assertEqual(
                observed.signature_binding_sha256,
                __import__("hashlib").sha256(certificate).hexdigest(),
            )

            bad = copy.deepcopy(json.loads(response))
            bad["thumbprint"] = "B" * 40
            rejected = subprocess.CompletedProcess(
                args=["powershell.exe"],
                returncode=0,
                stdout=json.dumps(bad),
                stderr="",
            )
            with (
                mock.patch.object(VALIDATOR.sys, "platform", "win32"),
                mock.patch.object(VALIDATOR.subprocess, "run", return_value=rejected),
                self.assertRaises(SystemExit),
            ):
                VALIDATOR.verify_windows_artifact(binary, WINDOWS_THUMBPRINT)


class ProductHardwareRunnerTests(unittest.TestCase):
    def test_driver_command_binds_every_independent_value(self) -> None:
        verified = artifact()
        command = RUNNER.driver_command(
            verified,
            pathlib.Path("/canonical/data"),
            source_sha=SOURCE_SHA,
            run_id=RUN_ID,
            platform="macos",
            topology="rotated",
        )
        self.assertEqual(command[0], str(verified.executable))
        for marker, value in (
            ("--frame-product-hardware-driver", None),
            ("--data-root", "/canonical/data"),
            ("--source-sha", SOURCE_SHA),
            ("--run-id", RUN_ID),
            ("--platform", "macos"),
            ("--topology", "rotated"),
            ("--signing-identity", MAC_TEAM),
            ("--binary-sha256", BINARY_SHA),
            ("--signature-binding-sha256", SIGNATURE_SHA),
            ("--application-id", VALIDATOR.APPLICATION_ID),
        ):
            with self.subTest(marker=marker):
                self.assertIn(marker, command)
                if value is not None:
                    self.assertEqual(command[command.index(marker) + 1], value)

    def test_output_path_must_be_new(self) -> None:
        with tempfile.TemporaryDirectory(prefix="frame-product-output-test-") as temporary:
            output = pathlib.Path(temporary) / "evidence.json"
            self.assertEqual(RUNNER.prepare_output_path(output), output.resolve())
            output.write_text("occupied", encoding="utf-8")
            with self.assertRaises(SystemExit):
                RUNNER.prepare_output_path(output)


if __name__ == "__main__":
    unittest.main()
