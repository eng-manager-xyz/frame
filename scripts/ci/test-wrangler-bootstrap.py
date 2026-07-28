#!/usr/bin/env python3
"""Prove local Wrangler conformance tolerates bounded registry stalls."""

from __future__ import annotations

import importlib.util
import pathlib
import subprocess
import sys
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[2]
RUNNERS = (
    ROOT / "scripts" / "ci" / "r2-storage-conformance.py",
    ROOT / "scripts" / "ci" / "d1-repository-conformance.py",
    ROOT / "scripts" / "ci" / "auth-d1-conformance.py",
)


def load_runner(path: pathlib.Path):
    module_name = f"frame_test_{path.stem.replace('-', '_')}"
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise AssertionError(f"could not load {path.name}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


class WranglerBootstrapTests(unittest.TestCase):
    def test_timeout_is_retried_before_accepting_the_exact_version(self) -> None:
        for path in RUNNERS:
            with self.subTest(runner=path.name):
                runner = load_runner(path)
                timeout = subprocess.TimeoutExpired(["wrangler", "--version"], 120)
                success = subprocess.CompletedProcess(
                    ["wrangler", "--version"],
                    returncode=0,
                    stdout=f"{runner.WRANGLER_VERSION}\n",
                    stderr="",
                )
                with (
                    mock.patch.object(
                        runner.subprocess,
                        "run",
                        side_effect=(timeout, success),
                    ) as run,
                    mock.patch.object(runner.time, "sleep") as sleep,
                ):
                    command = runner.detect_wrangler("/tmp/frame-wrangler")

                self.assertEqual(command, ["/tmp/frame-wrangler"])
                self.assertEqual(run.call_count, 2)
                self.assertEqual(
                    run.call_args_list[0].kwargs["timeout"],
                    runner.WRANGLER_BOOTSTRAP_TIMEOUT_SECONDS,
                )
                sleep.assert_called_once_with(
                    runner.WRANGLER_BOOTSTRAP_RETRY_DELAY_SECONDS
                )

    def test_wrong_version_exhausts_only_the_bounded_attempts(self) -> None:
        for path in RUNNERS:
            with self.subTest(runner=path.name):
                runner = load_runner(path)
                wrong = subprocess.CompletedProcess(
                    ["wrangler", "--version"],
                    returncode=0,
                    stdout="0.0.0\n",
                    stderr="",
                )
                with (
                    mock.patch.object(
                        runner.subprocess,
                        "run",
                        return_value=wrong,
                    ) as run,
                    mock.patch.object(runner.time, "sleep") as sleep,
                ):
                    with self.assertRaisesRegex(
                        runner.ConformanceFailure,
                        "could not be verified after 3 bounded attempts",
                    ):
                        runner.detect_wrangler("/tmp/frame-wrangler")

                self.assertEqual(run.call_count, runner.WRANGLER_BOOTSTRAP_ATTEMPTS)
                self.assertEqual(
                    sleep.call_count,
                    runner.WRANGLER_BOOTSTRAP_ATTEMPTS - 1,
                )


if __name__ == "__main__":
    unittest.main()
