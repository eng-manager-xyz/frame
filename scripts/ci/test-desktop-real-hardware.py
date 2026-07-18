#!/usr/bin/env python3
"""Credential-free unit tests for protected macOS hardware evidence validation."""

from __future__ import annotations

import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
import plistlib
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
VALIDATOR_PATH = ROOT / "scripts" / "ci" / "desktop-real-hardware.py"
SPEC = importlib.util.spec_from_file_location(
    "desktop_real_hardware", VALIDATOR_PATH
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load desktop real-hardware validator")
VALIDATOR = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = VALIDATOR
SPEC.loader.exec_module(VALIDATOR)

SOURCE_SHA = "a" * 40
RUN_ID = "123456:7"
TEAM_ID = "ABCDE12345"
EXECUTABLE_BYTES = b"signed frame executable\n"
EXECUTABLE_SHA256 = hashlib.sha256(EXECUTABLE_BYTES).hexdigest()
REQUIREMENT = (
    'designated => identifier "xyz.engmanager.frame" and anchor apple generic '
    'and certificate leaf[subject.OU] = "ABCDE12345"'
)
REQUIREMENT_SHA256 = hashlib.sha256(REQUIREMENT.encode("utf-8")).hexdigest()


def system_exit_message(callable_: object, *args: object, **kwargs: object) -> str:
    with unittest.TestCase().assertRaises(SystemExit) as raised:
        callable_(*args, **kwargs)  # type: ignore[operator]
    return str(raised.exception)


class SignedBundleTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="frame-hardware-test-")
        self.addCleanup(self.temporary.cleanup)
        self.app = Path(self.temporary.name) / "Frame.app"
        self.macos = self.app / "Contents" / "MacOS"
        self.macos.mkdir(parents=True)
        self.plist_path = self.app / "Contents" / "Info.plist"
        self.executable = self.macos / "frame-desktop"
        self.executable.write_bytes(EXECUTABLE_BYTES)
        self.executable.chmod(0o755)
        self.write_plist()

    def write_plist(
        self,
        *,
        bundle_id: object = VALIDATOR.MACOS_BUNDLE_IDENTIFIER,
        executable: object = "frame-desktop",
    ) -> None:
        with self.plist_path.open("wb") as target:
            plistlib.dump(
                {
                    "CFBundleIdentifier": bundle_id,
                    "CFBundleExecutable": executable,
                },
                target,
            )

    @staticmethod
    def signature_details(
        *,
        identifier: str = VALIDATOR.MACOS_BUNDLE_IDENTIFIER,
        team: str = TEAM_ID,
        signature: str = "size=4788",
    ) -> str:
        return (
            f"Identifier={identifier}\n"
            f"Signature={signature}\n"
            "Authority=Apple Development: Frame Tests\n"
            f"TeamIdentifier={team}\n"
        )

    def verify_with_outputs(
        self,
        details: str | None = None,
        requirement: str = REQUIREMENT,
        expected_team: str = TEAM_ID,
    ) -> tuple[tuple[str, str, str], mock.Mock]:
        outputs = (
            "valid on disk\nsatisfies its Designated Requirement\n",
            details if details is not None else self.signature_details(),
            "valid on disk\nsatisfies the test requirement\n",
            f"{requirement}\n",
        )
        with (
            mock.patch.object(VALIDATOR.sys, "platform", "darwin"),
            mock.patch.object(
                VALIDATOR, "command_output", side_effect=outputs
            ) as command,
        ):
            result = VALIDATOR.verify_signed_bundle(self.app, expected_team)
        return result, command

    def test_valid_certificate_signed_bundle_is_bound_to_executable(self) -> None:
        result, command = self.verify_with_outputs()

        self.assertEqual(
            result,
            (EXECUTABLE_SHA256, TEAM_ID, REQUIREMENT_SHA256),
        )
        self.assertEqual(
            command.call_args_list,
            [
                mock.call(
                    "codesign",
                    "--verify",
                    "--deep",
                    "--strict",
                    "--verbose=2",
                    str(self.app),
                ),
                mock.call(
                    "codesign",
                    "--display",
                    "--verbose=4",
                    str(self.app),
                ),
                mock.call(
                    "codesign",
                    "--verify",
                    "--deep",
                    "--strict",
                    "--verbose=2",
                    '-R=anchor apple generic and identifier '
                    '"xyz.engmanager.frame" and certificate '
                    'leaf[subject.OU] = "ABCDE12345"',
                    str(self.app),
                ),
                mock.call(
                    "codesign",
                    "--display",
                    "--requirements",
                    "-",
                    str(self.app),
                ),
            ],
        )

    def test_non_macos_host_is_rejected_before_bundle_or_codesign_access(self) -> None:
        with (
            mock.patch.object(VALIDATOR.sys, "platform", "linux"),
            mock.patch.object(VALIDATOR, "command_output") as command,
        ):
            message = system_exit_message(
                VALIDATOR.verify_signed_bundle, self.app, TEAM_ID
            )

        self.assertIn("requires macOS", message)
        command.assert_not_called()

    def test_bundle_and_plist_shape_are_checked_before_codesign(self) -> None:
        cases: list[tuple[str, object, str]] = [
            ("wrong id", "com.example.not-frame", "bundle id must be"),
            ("empty executable", "", "missing or unsafe"),
            ("traversal executable", "../frame-desktop", "missing or unsafe"),
            ("path executable", "nested/frame-desktop", "missing or unsafe"),
            ("non-string executable", 42, "missing or unsafe"),
            ("long executable", "x" * 129, "missing or unsafe"),
        ]
        for name, value, expected in cases:
            with self.subTest(name=name):
                if name == "wrong id":
                    self.write_plist(bundle_id=value)
                else:
                    self.write_plist(executable=value)
                with (
                    mock.patch.object(VALIDATOR.sys, "platform", "darwin"),
                    mock.patch.object(VALIDATOR, "command_output") as command,
                ):
                    message = system_exit_message(
                        VALIDATOR.verify_signed_bundle, self.app, TEAM_ID
                    )
                self.assertIn(expected, message)
                command.assert_not_called()
                self.write_plist()

    def test_invalid_plist_is_reported_as_a_controlled_failure(self) -> None:
        self.plist_path.write_bytes(b"this is not a plist")
        with (
            mock.patch.object(VALIDATOR.sys, "platform", "darwin"),
            mock.patch.object(VALIDATOR, "command_output") as command,
        ):
            message = system_exit_message(
                VALIDATOR.verify_signed_bundle, self.app, TEAM_ID
            )

        self.assertIn("Info.plist is unavailable", message)
        command.assert_not_called()

    def test_non_dictionary_plist_is_reported_as_a_controlled_failure(self) -> None:
        with self.plist_path.open("wb") as target:
            plistlib.dump(["not", "a", "dictionary"], target)
        with (
            mock.patch.object(VALIDATOR.sys, "platform", "darwin"),
            mock.patch.object(VALIDATOR, "command_output") as command,
        ):
            message = system_exit_message(
                VALIDATOR.verify_signed_bundle, self.app, TEAM_ID
            )

        self.assertIn("Info.plist root must be a dictionary", message)
        command.assert_not_called()

    def test_missing_directory_and_symlink_executables_are_rejected(self) -> None:
        self.executable.unlink()
        cases: list[tuple[str, object, str]] = [
            ("missing", None, "bundle executable is unavailable"),
            ("directory", "directory", "executable non-symlink regular file"),
            ("symlink", "symlink", "executable non-symlink regular file"),
        ]
        for name, replacement, expected in cases:
            with self.subTest(name=name):
                if replacement == "directory":
                    self.executable.mkdir()
                elif replacement == "symlink":
                    self.executable.symlink_to(self.plist_path)
                with (
                    mock.patch.object(VALIDATOR.sys, "platform", "darwin"),
                    mock.patch.object(VALIDATOR, "command_output") as command,
                ):
                    message = system_exit_message(
                        VALIDATOR.verify_signed_bundle, self.app, TEAM_ID
                    )
                self.assertIn(expected, message)
                command.assert_not_called()
                if self.executable.is_symlink() or self.executable.is_file():
                    self.executable.unlink()
                elif self.executable.is_dir():
                    self.executable.rmdir()

    def test_non_executable_regular_file_is_rejected_before_codesign(self) -> None:
        self.executable.chmod(0o644)
        with (
            mock.patch.object(VALIDATOR.sys, "platform", "darwin"),
            mock.patch.object(VALIDATOR, "command_output") as command,
        ):
            message = system_exit_message(
                VALIDATOR.verify_signed_bundle, self.app, TEAM_ID
            )

        self.assertIn("executable non-symlink regular file", message)
        command.assert_not_called()

    def test_symlink_app_bundle_is_rejected(self) -> None:
        alias = Path(self.temporary.name) / "Frame-alias.app"
        alias.symlink_to(self.app, target_is_directory=True)
        with (
            mock.patch.object(VALIDATOR.sys, "platform", "darwin"),
            mock.patch.object(VALIDATOR, "command_output") as command,
        ):
            message = system_exit_message(
                VALIDATOR.verify_signed_bundle, alias, TEAM_ID
            )

        self.assertIn("non-symlink directory", message)
        command.assert_not_called()

    def test_signature_identity_and_certificate_backing_are_enforced(self) -> None:
        cases = (
            (
                "identifier",
                self.signature_details(identifier="com.example.not-frame"),
                REQUIREMENT,
                TEAM_ID,
                "identifier does not match",
            ),
            (
                "ad hoc",
                self.signature_details(signature="adhoc"),
                REQUIREMENT,
                TEAM_ID,
                "ad-hoc signatures cannot satisfy",
            ),
            (
                "team",
                self.signature_details(team="OTHER12345"),
                REQUIREMENT,
                TEAM_ID,
                "team does not match",
            ),
            (
                "missing requirement",
                self.signature_details(),
                "identifier-only output",
                TEAM_ID,
                "no designated requirement",
            ),
            (
                "unanchored requirement",
                self.signature_details(),
                'designated => identifier "xyz.engmanager.frame"',
                TEAM_ID,
                "not certificate-backed and bundle-bound",
            ),
            (
                "anchor words only inside a value",
                self.signature_details(),
                'designated => identifier "xyz.engmanager.frame" and '
                'certificate leaf[subject.CN] = "anchor apple generic" and '
                'certificate leaf[subject.OU] = "ABCDE12345"',
                TEAM_ID,
                "not certificate-backed and bundle-bound",
            ),
            (
                "wrong requirement identifier",
                self.signature_details(),
                'designated => anchor apple and identifier "com.example.not-frame"',
                TEAM_ID,
                "not certificate-backed and bundle-bound",
            ),
        )
        for name, details, requirement, team, expected in cases:
            with self.subTest(name=name):
                with (
                    mock.patch.object(VALIDATOR.sys, "platform", "darwin"),
                    mock.patch.object(
                        VALIDATOR,
                        "command_output",
                        side_effect=(
                            "valid",
                            details,
                            "test requirement valid",
                            requirement,
                        ),
                    ),
                ):
                    message = system_exit_message(
                        VALIDATOR.verify_signed_bundle, self.app, team
                    )
                self.assertIn(expected, message)


class CommandOutputTests(unittest.TestCase):
    def test_command_output_returns_combined_codesign_streams(self) -> None:
        result = types.SimpleNamespace(
            returncode=0,
            stdout="stdout\n",
            stderr="stderr\n",
        )
        with mock.patch.object(VALIDATOR.subprocess, "run", return_value=result) as run:
            output = VALIDATOR.command_output("codesign", "--display", "Frame.app")

        self.assertEqual(output, "stdout\nstderr\n")
        run.assert_called_once_with(
            ("codesign", "--display", "Frame.app"),
            check=False,
            capture_output=True,
            text=True,
        )

    def test_codesign_failure_becomes_validator_failure(self) -> None:
        result = subprocess.CompletedProcess(
            args=("codesign", "--verify"),
            returncode=1,
            stdout="",
            stderr="code object is not signed at all\n",
        )
        with mock.patch.object(VALIDATOR.subprocess, "run", return_value=result):
            message = system_exit_message(
                VALIDATOR.command_output, "codesign", "--verify", "Frame.app"
            )

        self.assertIn("codesign --verify failed", message)
        self.assertIn("not signed", message)

    def test_metadata_value_requires_an_exact_line_key(self) -> None:
        details = "OtherIdentifier=wrong\nIdentifier=right\n"
        self.assertEqual(VALIDATOR.metadata_value(details, "Identifier"), "right")
        message = system_exit_message(
            VALIDATOR.metadata_value, details, "TeamIdentifier"
        )
        self.assertIn("does not report TeamIdentifier", message)


class EvidenceValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="frame-evidence-test-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.evidence_path = self.directory / "evidence.json"
        self.app = self.directory / "Frame.app"
        self.evidence = {
            "schema_version": 1,
            "evidence_class": "macos_display_capture_partial",
            "full_product_gate": "not_claimed",
            "capability": VALIDATOR.MACOS_DISPLAY_CAPABILITY,
            "platform": "macos",
            "adapter": VALIDATOR.MACOS_DISPLAY_ADAPTER,
            "source_sha": SOURCE_SHA,
            "run_id": RUN_ID,
            "bundle_identifier": VALIDATOR.MACOS_BUNDLE_IDENTIFIER,
            "signing_team_id": TEAM_ID,
            "binary_sha256": EXECUTABLE_SHA256,
            "designated_requirement_sha256": REQUIREMENT_SHA256,
            "cases": {case: True for case in VALIDATOR.MACOS_DISPLAY_CASES},
        }

    def arguments(self, *, require_hardware: bool = False) -> list[str]:
        arguments = [
            str(VALIDATOR_PATH),
            "--evidence",
            str(self.evidence_path),
            "--app-bundle",
            str(self.app),
            "--expected-source-sha",
            SOURCE_SHA,
            "--expected-run-id",
            RUN_ID,
            "--expected-signing-team",
            TEAM_ID,
            "--expected-capability",
            VALIDATOR.MACOS_DISPLAY_CAPABILITY,
        ]
        if require_hardware:
            arguments.append("--require-hardware")
        return arguments

    def invoke(
        self,
        evidence: object,
        *,
        require_hardware: bool = False,
        hardware_environment: str | None = None,
    ) -> tuple[int | None, str, str, mock.Mock]:
        return self.invoke_raw(
            json.dumps(evidence),
            require_hardware=require_hardware,
            hardware_environment=hardware_environment,
        )

    def invoke_raw(
        self,
        evidence: str,
        *,
        require_hardware: bool = False,
        hardware_environment: str | None = None,
    ) -> tuple[int | None, str, str, mock.Mock]:
        self.evidence_path.write_text(evidence, encoding="utf-8")
        stdout = io.StringIO()
        stderr = io.StringIO()
        environment = os.environ.copy()
        if hardware_environment is None:
            environment.pop("FRAME_REAL_HARDWARE", None)
        else:
            environment["FRAME_REAL_HARDWARE"] = hardware_environment
        with (
            mock.patch.object(VALIDATOR.sys, "argv", self.arguments(
                require_hardware=require_hardware
            )),
            mock.patch.dict(VALIDATOR.os.environ, environment, clear=True),
            mock.patch.object(
                VALIDATOR,
                "verify_signed_bundle",
                return_value=(EXECUTABLE_SHA256, TEAM_ID, REQUIREMENT_SHA256),
            ) as verify,
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            try:
                result = VALIDATOR.main()
            except SystemExit as error:
                result = None
                stderr.write(str(error))
        return result, stdout.getvalue(), stderr.getvalue(), verify

    def test_valid_evidence_passes_and_verifies_the_expected_bundle_team(self) -> None:
        result, stdout, stderr, verify = self.invoke(
            self.evidence,
            require_hardware=True,
            hardware_environment="1",
        )

        self.assertEqual(result, 0)
        self.assertEqual(stderr, "")
        self.assertIn("macOS display hardware evidence passed", stdout)
        self.assertIn(
            f"{len(VALIDATOR.PROTECTED_FULL_PRODUCT_CASES)}-case full-product gate",
            stdout,
        )
        verify.assert_called_once_with(self.app, TEAM_ID)

    def test_evidence_is_bound_to_every_independent_expectation(self) -> None:
        mutations = (
            ("schema_version", 2, "unsupported schema_version"),
            ("evidence_class", "full", "partial macOS display gate"),
            ("full_product_gate", "claimed", "must not claim"),
            ("capability", "other", "capability does not match"),
            ("platform", "windows", "only macOS is accepted"),
            ("adapter", "deterministic_fake", "adapter must be"),
            ("source_sha", "b" * 40, "source SHA does not match"),
            ("run_id", "other", "run id does not match"),
            ("bundle_identifier", "other", "bundle identifier does not match"),
            ("signing_team_id", "OTHER12345", "signing team does not match"),
            ("binary_sha256", "0" * 64, "binary digest does not match"),
            (
                "designated_requirement_sha256",
                "0" * 64,
                "designated requirement does not match",
            ),
        )
        for field, value, expected in mutations:
            with self.subTest(field=field):
                evidence = copy.deepcopy(self.evidence)
                evidence[field] = value
                result, _, stderr, verify = self.invoke(evidence)
                self.assertIsNone(result)
                self.assertIn(expected, stderr)
                verify.assert_called_once_with(self.app, TEAM_ID)

    def test_case_set_must_be_exact_and_every_case_true(self) -> None:
        cases = (
            (
                "missing",
                lambda evidence: evidence["cases"].pop("display_capture"),
                "missing=['display_capture']",
            ),
            (
                "unexpected",
                lambda evidence: evidence["cases"].update({"audio_capture": True}),
                "unexpected=['audio_capture']",
            ),
            (
                "false",
                lambda evidence: evidence["cases"].update({"display_capture": False}),
                "required cases did not pass",
            ),
            (
                "truthy string",
                lambda evidence: evidence["cases"].update({"display_capture": "true"}),
                "required cases did not pass",
            ),
        )
        for name, mutate, expected in cases:
            with self.subTest(name=name):
                evidence = copy.deepcopy(self.evidence)
                mutate(evidence)
                result, _, stderr, _ = self.invoke(evidence)
                self.assertIsNone(result)
                self.assertIn(expected, stderr)

    def test_non_object_evidence_is_rejected(self) -> None:
        result, _, stderr, _ = self.invoke([])
        self.assertIsNone(result)
        self.assertIn("evidence must be a JSON object", stderr)

    def test_boolean_schema_version_does_not_alias_integer_one(self) -> None:
        evidence = copy.deepcopy(self.evidence)
        evidence["schema_version"] = True
        result, _, stderr, _ = self.invoke(evidence)
        self.assertIsNone(result)
        self.assertIn("unsupported schema_version", stderr)

    def test_duplicate_json_keys_are_rejected_at_any_object_depth(self) -> None:
        top_level_duplicate = (
            '{"schema_version":1,"schema_version":1,"cases":{}}'
        )
        nested_duplicate = (
            '{"schema_version":1,"cases":{"display_capture":true,'
            '"display_capture":true}}'
        )
        for name, raw in (
            ("top level", top_level_duplicate),
            ("nested", nested_duplicate),
        ):
            with self.subTest(name=name):
                result, _, stderr, _ = self.invoke_raw(raw)
                self.assertIsNone(result)
                self.assertIn("duplicate JSON key", stderr)

    def test_require_hardware_needs_the_exact_protected_runner_marker(self) -> None:
        for marker in (None, "0", "true"):
            with self.subTest(marker=marker):
                result, _, stderr, verify = self.invoke(
                    self.evidence,
                    require_hardware=True,
                    hardware_environment=marker,
                )
                self.assertIsNone(result)
                self.assertIn("FRAME_REAL_HARDWARE=1 is required", stderr)
                verify.assert_not_called()


if __name__ == "__main__":
    unittest.main()
