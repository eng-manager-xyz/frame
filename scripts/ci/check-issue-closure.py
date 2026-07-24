#!/usr/bin/env python3
"""Validate the honest checkbox-by-checkbox closure inventory.

The checker never edits issue markdown and never promotes protected evidence.
Every checkbox is identified by its one-based ordinal within an issue. Any
unlisted ordinal is classified as locally satisfied; explicit protected and
local-gap sets take precedence and must be disjoint.
"""

from __future__ import annotations

import hashlib
import json
import pathlib
import re
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
INVENTORY = ROOT / "fixtures/closure/v1/checkbox-status.json"
ISSUES = ROOT / "_issues"
CHECKBOX = re.compile(r"^\s*- \[[ xX]\]")
ALLOWED_CLASSES = {
    "browser_accessibility_execution",
    "delivery_control_plane",
    "external_repository",
    "hardware_execution",
    "human_approval",
    "production_scale_data",
    "provider_execution",
}

# This is a narrow regression guard for the semantic audit recorded in
# fixtures/closure/v1/checkbox-status.json. It deliberately does not inspect
# implementation or infer closure; changing one of these sets requires a fresh
# human audit rather than compensating with an unrelated total adjustment.
AUDITED_LOCAL_GAPS = {
    "24": frozenset({4, 8}),
    "25": frozenset({1, 4, 5, 6, 7, 8}),
    "27": frozenset({2, 3, 4, 5, 8, 9, 10, 11}),
    "33": frozenset({3, 4, 5, 6, 8, 9, 10}),
}


class ClosureFailure(RuntimeError):
    """Stable validation failure without repository contents or secrets."""


def load_json(path: pathlib.Path) -> dict[str, Any]:
    def reject_duplicate(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ClosureFailure(f"duplicate JSON key in {path.relative_to(ROOT)}")
            result[key] = value
        return result

    try:
        value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=reject_duplicate)
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ClosureFailure(f"cannot decode {path.relative_to(ROOT)}") from error
    if not isinstance(value, dict):
        raise ClosureFailure("closure inventory root must be an object")
    return value


def issue_files() -> dict[str, pathlib.Path]:
    result: dict[str, pathlib.Path] = {}
    for path in sorted(ISSUES.glob("[0-9][0-9]-*.md")):
        issue = path.name[:2]
        if issue in result:
            raise ClosureFailure(f"duplicate issue number {issue}")
        result[issue] = path
    if list(result) != [f"{number:02d}" for number in range(1, 45)]:
        raise ClosureFailure("expected the contiguous issue range 01 through 44")
    return result


def checkbox_items(path: pathlib.Path) -> list[str]:
    """Return complete normalized checkbox text, including wrapped lines."""
    result: list[str] = []
    current: list[str] | None = None
    for line in path.read_text(encoding="utf-8").splitlines():
        if CHECKBOX.match(line):
            if current is not None:
                result.append(" ".join(current))
            current = [line.strip()]
        elif current is not None and line.startswith("  ") and line.strip():
            current.append(line.strip())
        elif current is not None:
            result.append(" ".join(current))
            current = None
    if current is not None:
        result.append(" ".join(current))
    return result


def ordinal_set(groups: object, issue: str, field: str, count: int) -> set[int]:
    if not isinstance(groups, list):
        raise ClosureFailure(f"issue {issue} {field} must be an array")
    result: set[int] = set()
    for group in groups:
        if not isinstance(group, dict) or set(group) != ({"ordinals", "classes", "reason"} if field == "protected" else {"ordinals", "reason"}):
            raise ClosureFailure(f"issue {issue} has an invalid {field} group shape")
        ordinals = group.get("ordinals")
        reason = group.get("reason")
        if not isinstance(ordinals, list) or not ordinals or not isinstance(reason, str) or len(reason) < 24:
            raise ClosureFailure(f"issue {issue} has an invalid {field} group")
        if field == "protected":
            classes = group.get("classes")
            if not isinstance(classes, list) or not classes or any(value not in ALLOWED_CLASSES for value in classes):
                raise ClosureFailure(f"issue {issue} has an invalid protected class")
            if len(classes) != len(set(classes)):
                raise ClosureFailure(f"issue {issue} repeats a protected class")
        for ordinal in ordinals:
            if not isinstance(ordinal, int) or isinstance(ordinal, bool) or not 1 <= ordinal <= count:
                raise ClosureFailure(f"issue {issue} has an out-of-range {field} ordinal")
            if ordinal in result:
                raise ClosureFailure(f"issue {issue} repeats {field} ordinal {ordinal}")
            result.add(ordinal)
    return result


def evidence_paths(value: object, issue: str) -> None:
    if not isinstance(value, list) or not value:
        raise ClosureFailure(f"issue {issue} must name repository evidence")
    for item in value:
        if (
            not isinstance(item, str)
            or item.startswith("/")
            or any(part in {"", ".", ".."} for part in pathlib.PurePosixPath(item).parts)
        ):
            raise ClosureFailure(f"issue {issue} has an unsafe evidence path")
        path = ROOT / item
        if not path.exists() or path.is_symlink() or not path.is_file():
            raise ClosureFailure(f"issue {issue} evidence path is absent or unsafe: {item}")


def main() -> int:
    inventory = load_json(INVENTORY)
    if inventory.get("schema_version") != 1:
        raise ClosureFailure("unsupported closure inventory schema")
    expected = inventory.get("expected_totals")
    records = inventory.get("issues")
    if not isinstance(expected, dict) or not isinstance(records, dict):
        raise ClosureFailure("closure inventory is missing totals or issues")

    files = issue_files()
    if set(records) != set(files):
        raise ClosureFailure("closure inventory must cover exactly issues 01 through 44")

    totals = {"checkboxes": 0, "local_satisfied": 0, "protected_pending": 0, "local_gap": 0}
    gaps: list[tuple[str, int, str]] = []
    protected_classes: dict[str, int] = {name: 0 for name in ALLOWED_CLASSES}

    for issue, path in files.items():
        record = records[issue]
        if not isinstance(record, dict) or set(record) != {"count", "digest", "evidence", "protected", "local_gaps"}:
            raise ClosureFailure(f"issue {issue} has an invalid record shape")
        lines = checkbox_items(path)
        count = record.get("count")
        if count != len(lines):
            raise ClosureFailure(f"issue {issue} checkbox count drifted")
        digest = hashlib.sha256("".join(f"{line}\n" for line in lines).encode()).hexdigest()
        if record.get("digest") != digest:
            raise ClosureFailure(f"issue {issue} checkbox text drifted; re-audit classifications")
        evidence_paths(record.get("evidence"), issue)

        protected = ordinal_set(record.get("protected"), issue, "protected", count)
        local_gaps = ordinal_set(record.get("local_gaps"), issue, "local_gaps", count)
        if issue in AUDITED_LOCAL_GAPS and local_gaps != AUDITED_LOCAL_GAPS[issue]:
            raise ClosureFailure(
                f"issue {issue} audited local-gap ordinals drifted; rerun the semantic audit"
            )
        overlap = protected & local_gaps
        if overlap:
            raise ClosureFailure(f"issue {issue} classifies ordinals twice")
        local = set(range(1, count + 1)) - protected - local_gaps

        for group in record["protected"]:
            for class_name in group["classes"]:
                protected_classes[class_name] += len(group["ordinals"])
        for ordinal in sorted(local_gaps):
            gaps.append((issue, ordinal, lines[ordinal - 1][lines[ordinal - 1].find("]") + 1 :].strip()))

        totals["checkboxes"] += count
        totals["local_satisfied"] += len(local)
        totals["protected_pending"] += len(protected)
        totals["local_gap"] += len(local_gaps)

    if totals != expected:
        raise ClosureFailure(f"closure totals drifted: {totals!r}")
    if sum(totals[name] for name in ("local_satisfied", "protected_pending", "local_gap")) != totals["checkboxes"]:
        raise ClosureFailure("closure classifications do not partition every checkbox")

    print(
        "issue closure audit passed: "
        f"{totals['checkboxes']} checkboxes; "
        f"{totals['local_satisfied']} local-satisfied, "
        f"{totals['protected_pending']} protected-pending, "
        f"{totals['local_gap']} true local gaps"
    )
    print("protected class memberships (compound checkboxes may count in more than one class):")
    for name in sorted(protected_classes):
        print(f"  {name}: {protected_classes[name]}")
    print("true local gaps:")
    for issue, ordinal, text in gaps:
        print(f"  issue {issue} checkbox {ordinal}: {text}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ClosureFailure as error:
        print(f"issue closure audit failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
