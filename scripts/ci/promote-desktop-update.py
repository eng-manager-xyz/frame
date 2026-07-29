#!/usr/bin/env python3
"""Promote one signed desktop pointer while preserving one rollback release."""

from __future__ import annotations

import argparse
import json
import re
import tempfile
from pathlib import Path
from typing import Any


MAX_ARTIFACT_BYTES = 512 * 1024 * 1024
MAX_NOTES_BYTES = 8_192
STABLE_VERSION = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$"
)
POINTER_KEYS = {
    "schema_version",
    "version",
    "signature",
    "bytes",
    "sha256",
    "notes",
    "pub_date",
}


class PromotionError(ValueError):
    pass


def fail(message: str) -> None:
    raise PromotionError(message)


def load_pointer(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise PromotionError("pointer is unreadable") from error
    if not isinstance(value, dict) or set(value) != POINTER_KEYS:
        fail("pointer schema is not exact")
    version = value["version"]
    signature = value["signature"]
    size = value["bytes"]
    sha256 = value["sha256"]
    notes = value["notes"]
    pub_date = value["pub_date"]
    if value["schema_version"] != 1:
        fail("pointer schema version is unsupported")
    if not isinstance(version, str) or not STABLE_VERSION.fullmatch(version):
        fail("pointer version is not stable canonical semver")
    if (
        not isinstance(signature, str)
        or not signature.isascii()
        or not 32 <= len(signature) <= 2_048
        or any(character.isspace() for character in signature)
    ):
        fail("pointer signature is invalid")
    if (
        not isinstance(size, int)
        or isinstance(size, bool)
        or not 1 <= size <= MAX_ARTIFACT_BYTES
    ):
        fail("pointer size is invalid")
    if (
        not isinstance(sha256, str)
        or len(sha256) != 64
        or any(character not in "0123456789abcdef" for character in sha256)
    ):
        fail("pointer digest is invalid")
    if notes is not None and (
        not isinstance(notes, str)
        or len(notes.encode("utf-8")) > MAX_NOTES_BYTES
        or "\0" in notes
    ):
        fail("pointer notes are invalid")
    if pub_date is not None and (
        not isinstance(pub_date, str)
        or not pub_date.isascii()
        or len(pub_date) > 64
        or any(character.isspace() for character in pub_date)
    ):
        fail("pointer publication date is invalid")
    return value


def version_tuple(pointer: dict[str, Any]) -> tuple[int, int, int]:
    return tuple(int(part) for part in pointer["version"].split("."))  # type: ignore[return-value]


def promotion(
    current: dict[str, Any] | None, candidate: dict[str, Any]
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    if current is None:
        return candidate, None
    current_version = version_tuple(current)
    candidate_version = version_tuple(candidate)
    if candidate_version < current_version:
        fail("candidate release is older than the current release")
    if candidate_version == current_version:
        if candidate != current:
            fail("release version was reused with different signed metadata")
        return candidate, None
    return candidate, current


def write_pointer(path: Path, pointer: dict[str, Any]) -> None:
    path.write_text(
        json.dumps(pointer, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def self_test() -> None:
    base: dict[str, Any] = {
        "schema_version": 1,
        "version": "1.2.3",
        "signature": "A" * 64,
        "bytes": 1_024,
        "sha256": "a" * 64,
        "notes": None,
        "pub_date": "2026-07-29T12:00:00Z",
    }
    candidate = {**base, "version": "1.2.4", "signature": "B" * 64}
    latest, previous = promotion(base, candidate)
    assert latest == candidate
    assert previous == base
    assert promotion(candidate, candidate) == (candidate, None)
    try:
        promotion(candidate, {**candidate, "sha256": "b" * 64})
    except PromotionError:
        pass
    else:
        raise AssertionError("reused version accepted different signed metadata")
    try:
        promotion(candidate, base)
    except PromotionError:
        pass
    else:
        raise AssertionError("rollback candidate replaced the latest pointer")
    with tempfile.TemporaryDirectory(prefix="frame-update-promotion-") as temporary:
        pointer_path = Path(temporary) / "pointer.json"
        write_pointer(pointer_path, candidate)
        assert load_pointer(pointer_path) == candidate


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--current", type=Path)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--latest-output", type=Path)
    parser.add_argument("--previous-output", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("desktop updater promotion contract passed")
        return 0
    if args.candidate is None or args.latest_output is None:
        parser.error("--candidate and --latest-output are required")
    try:
        current = load_pointer(args.current) if args.current is not None else None
        candidate = load_pointer(args.candidate)
        latest, previous = promotion(current, candidate)
        write_pointer(args.latest_output, latest)
        if previous is not None:
            if args.previous_output is None:
                fail("--previous-output is required for a newer promotion")
            write_pointer(args.previous_output, previous)
        elif args.previous_output is not None:
            args.previous_output.unlink(missing_ok=True)
    except (OSError, PromotionError) as error:
        raise SystemExit(f"desktop update promotion failed: {error}") from error
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
