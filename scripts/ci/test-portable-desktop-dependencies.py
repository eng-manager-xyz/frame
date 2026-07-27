#!/usr/bin/env python3
"""Unit tests for the portable desktop dependency-closure checker."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CHECKER_PATH = ROOT / "scripts" / "ci" / "check-portable-desktop-dependencies.py"
SPEC = importlib.util.spec_from_file_location(
    "check_portable_desktop_dependencies", CHECKER_PATH
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load portable dependency checker")
CHECKER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CHECKER
SPEC.loader.exec_module(CHECKER)


class PortableDependencyCheckerTests(unittest.TestCase):
    def test_accepts_portable_tauri_dependencies(self) -> None:
        tree = "\n".join(
            (
                "frame-desktop-core v0.1.0 (/checkout/apps/desktop)",
                "tauri v2.9.5",
                "serde v1.0.228",
            )
        )
        self.assertEqual(CHECKER.denied_dependencies(tree), [])

    def test_rejects_each_native_frame_crate(self) -> None:
        names = (
            "frame-media",
            "frame-platform-lifecycle",
            "frame-macos-screen-capture",
            "frame-macos-av-capture",
            "frame-windows-screen-capture",
            "frame-windows-capture-ffi",
            "wgc",
        )
        for name in names:
            with self.subTest(name=name):
                self.assertEqual(
                    CHECKER.denied_dependencies(f"{name} v1.2.3"),
                    [(1, f"{name} v1.2.3")],
                )

    def test_rejects_gstreamer_crate_variants(self) -> None:
        tree = "gstreamer v0.24.4\ngstreamer-sys v0.24.4\n"
        self.assertEqual(
            CHECKER.denied_dependencies(tree),
            [(1, "gstreamer v0.24.4"), (2, "gstreamer-sys v0.24.4")],
        )

    def test_strips_forced_cargo_color_before_matching(self) -> None:
        tree = "\x1b[32mframe-media\x1b[0m \x1b[36mv0.1.0\x1b[0m\n"
        self.assertEqual(
            CHECKER.denied_dependencies(tree),
            [(1, "frame-media v0.1.0")],
        )


if __name__ == "__main__":
    unittest.main()
