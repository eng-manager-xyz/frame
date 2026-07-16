#!/usr/bin/env python3
"""Validate the provider-free share/player contract and its evidence boundary."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re


ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures/web-share-player/v1/contract.json"
SOURCE = ROOT / "apps/web/src/share_player.rs"
PAGES = ROOT / "apps/web/src/pages.rs"
HYDRATION = ROOT / "apps/web/src/hydration.rs"
ARCHITECTURE = ROOT / "docs/architecture/share-player-v1.md"
EVIDENCE = ROOT / "docs/evidence/share-player-local.md"
CAP_COMMIT = "6ba69561ac86b8efdb17616d6727f9638015546b"
SAFE_CASE = re.compile(r"^[a-z][a-z0-9_]{2,63}$")
EXPECTED_PRIVACY = {"public", "unlisted", "tenant", "private", "password"}
EXPECTED_LIFECYCLE = {"ready", "processing", "failed", "deleted", "unavailable"}
EXPECTED_COMPATIBILITY = {
    "canonical_share",
    "legacy_share",
    "canonical_embed",
    "custom_domain",
    "cache",
}
EXPECTED_BROWSERS = {
    ("chromium", "desktop"),
    ("firefox", "desktop"),
    ("webkit", "desktop"),
    ("mobile_safari", "mobile"),
    ("mobile_chromium", "mobile"),
}
EXPECTED_PROTECTED = {
    "production_share_authorization_adapter",
    "d1_comment_transcript_analytics_adapters",
    "r2_range_and_revocation_traces",
    "verified_custom_domain_routing",
    "password_hash_and_delivery_service",
    "browser_device_accessibility_execution",
    "cdn_privacy_invalidation_slo",
    "canary_and_rollback_observation",
}


def fail(message: str) -> "None":
    raise SystemExit(f"share/player contract invalid: {message}")


def load_json(path: pathlib.Path) -> tuple[dict[str, object], bytes]:
    if path.is_symlink():
        fail(f"refusing symlink: {path.relative_to(ROOT)}")
    raw = path.read_bytes()
    if not raw or len(raw) > 250_000 or b"\x00" in raw:
        fail("fixture size or encoding is unsafe")
    value = json.loads(raw)
    if not isinstance(value, dict):
        fail("fixture top level must be an object")
    return value, raw


def exact_keys(value: dict[str, object], expected: set[str], label: str) -> None:
    actual = set(value)
    if actual != expected:
        fail(
            f"{label} keys differ: missing={sorted(expected - actual)} "
            f"extra={sorted(actual - expected)}"
        )


def validate_matrix(value: dict[str, object]) -> None:
    matrix = value["authorization_matrix"]
    if not isinstance(matrix, list) or len(matrix) < 18:
        fail("authorization matrix is incomplete")
    cases: set[str] = set()
    privacy: set[str] = set()
    lifecycle: set[str] = set()
    for index, row in enumerate(matrix):
        if not isinstance(row, dict):
            fail(f"authorization row {index} is not an object")
        exact_keys(
            row,
            {"case", "privacy", "lifecycle", "viewer", "surface", "resolution", "status", "metadata"},
            f"authorization row {index}",
        )
        case = row["case"]
        if not isinstance(case, str) or SAFE_CASE.fullmatch(case) is None or case in cases:
            fail(f"authorization row {index} has an unsafe or duplicate case")
        cases.add(case)
        if not isinstance(row["privacy"], str) or not isinstance(row["lifecycle"], str):
            fail(f"authorization row {case} has non-string state")
        privacy.add(row["privacy"])
        lifecycle.add(row["lifecycle"])
        resolution = row["resolution"]
        status = row["status"]
        if resolution not in {"ready", "processing", "password_challenge", "unavailable"}:
            fail(f"authorization row {case} has unknown resolution")
        expected_status = {
            "ready": 200,
            "processing": 202,
            "password_challenge": 401,
            "unavailable": 404,
        }[resolution]
        if status != expected_status:
            fail(f"authorization row {case} status does not match its resolution")
        if resolution != "ready" and row["metadata"] != "generic":
            fail(f"authorization row {case} can leak non-ready metadata")
    if privacy != EXPECTED_PRIVACY or lifecycle != EXPECTED_LIFECYCLE:
        fail("authorization matrix does not cover every privacy/lifecycle state")


def validate_range(value: dict[str, object]) -> None:
    rows = value["range_cases"]
    if not isinstance(rows, list) or len(rows) != 8:
        fail("range matrix must contain exactly eight closed cases")
    required = {"full", "partial_10_19", "partial_90_99", "unsatisfiable"}
    resolutions: set[str] = set()
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            fail(f"range row {index} is not an object")
        exact_keys(row, {"header", "if_range_matches", "length", "resolution"}, f"range row {index}")
        if row["header"] is not None and not isinstance(row["header"], str):
            fail(f"range row {index} header is invalid")
        if not isinstance(row["if_range_matches"], bool) or row["length"] != 100:
            fail(f"range row {index} preconditions drifted")
        if not isinstance(row["resolution"], str):
            fail(f"range row {index} resolution is invalid")
        resolutions.add(row["resolution"])
    if resolutions != required:
        fail("range matrix lost a required resolution")


def validate_compatibility(value: dict[str, object]) -> None:
    rows = value["compatibility"]
    if not isinstance(rows, list):
        fail("compatibility matrix must be a list")
    surfaces: set[str] = set()
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            fail(f"compatibility row {index} is not an object")
        exact_keys(row, {"surface", "input", "result"}, f"compatibility row {index}")
        if any(not isinstance(row[field], str) for field in ("surface", "input", "result")):
            fail(f"compatibility row {index} fields must be strings")
        surfaces.add(row["surface"])
    if surfaces != EXPECTED_COMPATIBILITY:
        fail("compatibility matrix surface coverage drifted")


def validate_browser_and_protected(value: dict[str, object]) -> None:
    rows = value["browser_matrix"]
    if not isinstance(rows, list):
        fail("browser matrix must be a list")
    found: set[tuple[str, str]] = set()
    for index, row in enumerate(rows):
        if not isinstance(row, dict):
            fail(f"browser row {index} is not an object")
        exact_keys(row, {"browser", "platform", "expected", "evidence"}, f"browser row {index}")
        browser, platform = row["browser"], row["platform"]
        if not isinstance(browser, str) or not isinstance(platform, str):
            fail(f"browser row {index} identity is invalid")
        found.add((browser, platform))
        if row["evidence"] != "protected_execution_pending":
            fail(f"browser row {browser} overclaims protected execution")
        expected = row["expected"]
        if not isinstance(expected, list) or len(expected) < 6 or any(not isinstance(item, str) for item in expected):
            fail(f"browser row {browser} expected checks are incomplete")
    if found != EXPECTED_BROWSERS:
        fail("browser/device matrix coverage drifted")
    protected = value["protected_evidence"]
    if not isinstance(protected, list) or set(protected) != EXPECTED_PROTECTED or len(protected) != len(EXPECTED_PROTECTED):
        fail("protected evidence inventory drifted")


def require_markers(path: pathlib.Path, markers: tuple[str, ...]) -> str:
    if path.is_symlink():
        fail(f"refusing symlink: {path.relative_to(ROOT)}")
    text = path.read_text(encoding="utf-8")
    missing = [marker for marker in markers if marker not in text]
    if missing:
        fail(f"{path.relative_to(ROOT)} missing markers: {missing}")
    return text


def validate_sources() -> None:
    source = require_markers(
        SOURCE,
        (
            "pub fn resolve_share",
            "pub fn summary_is_scope_safe",
            "pub fn plan_byte_range",
            "pub struct EmbedSession",
            "pub fn validate_comment",
            "pub fn validate_analytics",
            "impl TranscriptDocument",
            "private, no-store, max-age=0",
        ),
    )
    for forbidden in ("X-Amz-Signature", "cloudflare.com", "r2.cloudflarestorage.com"):
        if forbidden in source:
            fail(f"share contract contains provider or bearer marker {forbidden}")
    require_markers(
        PAGES,
        (
            'id="frame-public-player"',
            'controlslist="nodownload noremoteplayback"',
            'data-allow-picture-in-picture',
            '"Analytics stay off unless',
            '"Comments"',
            '"Transcript"',
        ),
    )
    require_markers(
        HYDRATION,
        (
            '"Play or pause"',
            '"Back 10 seconds"',
            '"Forward 10 seconds"',
            '"Retry playback"',
            "serde_wasm_bindgen::from_value::<EmbedCommandEnvelope>",
            "source_is_parent",
        ),
    )
    architecture = require_markers(
        ARCHITECTURE,
        tuple(sorted(EXPECTED_PROTECTED)),
    )
    evidence = require_markers(
        EVIDENCE,
        (
            "provider-free local evidence only",
            "58 tests",
            "protected_execution_pending",
            "cargo clippy -p frame-web",
        ),
    )
    if "all acceptance criteria complete" in architecture.lower() or "production complete" in evidence.lower():
        fail("documentation overclaims production completion")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path)
    arguments = parser.parse_args()
    value, raw = load_json(FIXTURE)
    exact_keys(
        value,
        {
            "schema_version",
            "contract_version",
            "cap_reference_commit",
            "authorization_matrix",
            "range_cases",
            "compatibility",
            "browser_matrix",
            "protected_evidence",
        },
        "contract",
    )
    if value["schema_version"] != 1 or value["contract_version"] != 1:
        fail("unsupported contract version")
    if value["cap_reference_commit"] != CAP_COMMIT:
        fail("Cap reference commit drifted")
    validate_matrix(value)
    validate_range(value)
    validate_compatibility(value)
    validate_browser_and_protected(value)
    validate_sources()
    result = {
        "schema": "frame.share-player-local-evidence.v1",
        "contract_sha256": hashlib.sha256(raw).hexdigest(),
        "authorization_cases": len(value["authorization_matrix"]),
        "range_cases": len(value["range_cases"]),
        "browser_rows": len(value["browser_matrix"]),
        "protected_evidence": len(value["protected_evidence"]),
        "result": "pass",
        "scope": "provider_free_local_contract",
    }
    serialized = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if arguments.evidence is not None:
        evidence = arguments.evidence.resolve()
        target = (ROOT / "target/evidence").resolve()
        if not evidence.is_relative_to(target):
            fail("--evidence must be below target/evidence")
        evidence.parent.mkdir(parents=True, exist_ok=True)
        evidence.write_text(serialized, encoding="utf-8")
    print(serialized, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
