#!/usr/bin/env python3
"""Validate the immutable, privacy-safe local parity evidence corpus."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import pathlib
import re
import statistics
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
CORPUS = ROOT / "fixtures" / "parity" / "v1"
MANIFEST = CORPUS / "manifest.json"

REQUIRED_FLOWS = {"auth", "tenant", "upload", "media", "share", "cutover"}
REQUIRED_SCENARIO_CLASSES = {"happy", "failure"}
ALLOWED_EVIDENCE_CLASSES = {
    "local_simulated",
    "protected_placeholder",
    "test_definition",
}
FORBIDDEN_KEYS = {
    "absolute_path",
    "authorization",
    "binary",
    "bytes_base64",
    "cookie",
    "customer_data",
    "customer_id",
    "email",
    "object_key",
    "password",
    "raw_media",
    "secret",
    "signed_url",
    "media_base64",
    "payload_base64",
    "source_file",
    "token",
    "user_id",
}
FORBIDDEN_VALUE_MARKERS = (
    "-----begin private key-----",
    "authorization:",
    "aws_secret_access_key",
    "bearer ",
    "data:audio/",
    "data:image/",
    "data:video/",
    "frame-recordings/",
    "r2.cloudflarestorage.com",
    "x-amz-credential=",
    "x-amz-signature=",
)
EMAIL_PATTERN = re.compile(r"(?i)\b[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9.-]+\.[a-z]{2,}\b")
SHA256_PATTERN = re.compile(r"[0-9a-f]{64}")


class DuplicateKeyError(ValueError):
    """Raised when JSON contains an ambiguous duplicate object key."""


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise DuplicateKeyError(f"duplicate key {key!r}")
        result[key] = value
    return result


def display_path(path: pathlib.Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return "<outside-repository>"


def load_json(path: pathlib.Path, errors: list[str]) -> Any | None:
    try:
        return json.loads(
            path.read_text(encoding="utf-8"), object_pairs_hook=reject_duplicate_keys
        )
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, DuplicateKeyError) as error:
        errors.append(f"{display_path(path)}: invalid JSON ({type(error).__name__})")
        return None


def require_mapping(value: Any, label: str, errors: list[str]) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        errors.append(f"{label}: expected an object")
        return None
    return value


def walk_safe_content(value: Any, label: str, errors: list[str]) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            normalized = key.lower().replace("-", "_")
            if normalized in FORBIDDEN_KEYS:
                errors.append(f"{label}: forbidden sensitive key {key!r}")
            walk_safe_content(child, f"{label}.{key}", errors)
    elif isinstance(value, list):
        if len(value) >= 32 and all(
            isinstance(child, int) and not isinstance(child, bool) and 0 <= child <= 255
            for child in value
        ):
            errors.append(f"{label}: contains a byte-like payload instead of metadata")
        for index, child in enumerate(value):
            walk_safe_content(child, f"{label}[{index}]", errors)
    elif isinstance(value, str):
        lowered = value.lower()
        if len(value) > 512:
            errors.append(f"{label}: string is too large for metadata-only evidence")
        if EMAIL_PATTERN.search(value):
            errors.append(f"{label}: contains an email-like value")
        if any(marker in lowered for marker in FORBIDDEN_VALUE_MARKERS):
            errors.append(f"{label}: contains a secret-bearing or internal marker")
        if lowered.startswith(("/users/", "c:\\users\\")):
            errors.append(f"{label}: contains a workstation-specific home path")


def validate_manifest(manifest: Any, errors: list[str]) -> dict[str, Any] | None:
    manifest = require_mapping(manifest, "manifest", errors)
    if manifest is None:
        return None
    if manifest.get("schema_version") != 1:
        errors.append("manifest: schema_version must be 1")
    if manifest.get("corpus_version") != "parity-v1":
        errors.append("manifest: corpus_version must be parity-v1")
    if manifest.get("manifest_kind") != "immutable_parity_fixture_manifest":
        errors.append("manifest: manifest_kind is invalid")
    if manifest.get("immutable") is not True:
        errors.append("manifest: immutable must be true")

    cap_reference = manifest.get("cap_reference")
    if not isinstance(cap_reference, dict):
        errors.append("manifest: cap_reference is required")
    else:
        if cap_reference.get("repository") != "CapSoftware/Cap":
            errors.append("manifest: Cap repository reference is invalid")
        commit = cap_reference.get("commit_sha")
        if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit):
            errors.append("manifest: Cap commit must be a full lowercase SHA")
        if cap_reference.get("artifacts_copied") is not False:
            errors.append("manifest: upstream artifacts must not be copied")

    provenance = manifest.get("provenance")
    if not isinstance(provenance, dict):
        errors.append("manifest: provenance is required")
    else:
        if provenance.get("origin") != "deterministic_generated_metadata":
            errors.append("manifest: only deterministic generated metadata is allowed")
        if provenance.get("contains_upstream_media") is not False:
            errors.append("manifest: upstream media is forbidden")
        if provenance.get("contains_customer_media") is not False:
            errors.append("manifest: customer media is forbidden")

    redistribution = manifest.get("redistribution")
    if not isinstance(redistribution, dict):
        errors.append("manifest: redistribution declaration is required")
    elif (
        redistribution.get("allowed") is not True
        or redistribution.get("third_party_assets") is not False
        or redistribution.get("scope") != "generated_json_metadata_only"
    ):
        errors.append("manifest: redistribution declaration is not safe")

    privacy = manifest.get("privacy")
    if not isinstance(privacy, dict):
        errors.append("manifest: privacy declaration is required")
    else:
        required_false = {
            "contains_customer_data",
            "contains_personal_data",
            "contains_secrets",
            "contains_media_bytes",
        }
        for field in sorted(required_false):
            if privacy.get(field) is not False:
                errors.append(f"manifest: privacy.{field} must be false")
        if privacy.get("review_status") != "synthetic_metadata_only":
            errors.append("manifest: privacy review_status is invalid")

    lanes = manifest.get("lanes")
    if not isinstance(lanes, dict) or set(lanes) != {"fast", "full"}:
        errors.append("manifest: exactly fast and full lanes are required")
    else:
        fast = lanes.get("fast")
        full = lanes.get("full")
        if not isinstance(fast, dict) or not isinstance(full, dict):
            errors.append("manifest: lane declarations must be objects")
        else:
            if fast.get("evidence_scope") != "local_simulated_only":
                errors.append("manifest: fast lane must be explicitly local simulated")
            if fast.get("promotion_claim") != "local_contract_gate_only":
                errors.append("manifest: fast lane promotion claim is invalid")
            if full.get("extends") != "fast":
                errors.append("manifest: full lane must extend fast")
            if full.get("evidence_scope") != "protected_provider_and_hardware_required":
                errors.append("manifest: full lane must require protected evidence")
            if full.get("promotion_claim") != "blocked_until_protected_records_are_collected":
                errors.append("manifest: full lane must remain blocked by placeholders")
    return manifest


def validate_file_inventory(
    manifest: dict[str, Any], errors: list[str]
) -> tuple[dict[str, Any], int]:
    entries = manifest.get("files")
    if not isinstance(entries, list) or not entries:
        errors.append("manifest: files must be a non-empty list")
        return {}, 0

    parsed: dict[str, Any] = {}
    listed_paths: list[str] = []
    for index, entry in enumerate(entries):
        label = f"manifest.files[{index}]"
        if not isinstance(entry, dict):
            errors.append(f"{label}: expected an object")
            continue
        relative = entry.get("path")
        if not isinstance(relative, str):
            errors.append(f"{label}: path must be a string")
            continue
        pure_path = pathlib.PurePosixPath(relative)
        if pure_path.is_absolute() or ".." in pure_path.parts or relative == "manifest.json":
            errors.append(f"{label}: unsafe or reserved path")
            continue
        listed_paths.append(relative)
        if entry.get("media_type") != "application/json":
            errors.append(f"{label}: only application/json fixtures are allowed")
        if entry.get("evidence_class") not in ALLOWED_EVIDENCE_CLASSES:
            errors.append(f"{label}: evidence_class is invalid")

        expected_hash = entry.get("sha256")
        if not isinstance(expected_hash, str) or not SHA256_PATTERN.fullmatch(expected_hash):
            errors.append(f"{label}: sha256 must be a lowercase SHA-256 digest")
            continue
        expected_bytes = entry.get("bytes")
        if not isinstance(expected_bytes, int) or isinstance(expected_bytes, bool) or expected_bytes < 2:
            errors.append(f"{label}: bytes must be a positive integer")
            continue

        path = CORPUS / pathlib.Path(*pure_path.parts)
        if path.is_symlink():
            errors.append(f"{label}: symlinks are not allowed")
            continue
        try:
            payload_bytes = path.read_bytes()
        except OSError:
            errors.append(f"{label}: fixture is missing or unreadable")
            continue
        if len(payload_bytes) != expected_bytes:
            errors.append(f"{label}: byte count does not match immutable manifest")
        if hashlib.sha256(payload_bytes).hexdigest() != expected_hash:
            errors.append(f"{label}: SHA-256 does not match immutable manifest")
        payload = load_json(path, errors)
        if payload is not None:
            parsed[relative] = payload

    if listed_paths != sorted(listed_paths) or len(listed_paths) != len(set(listed_paths)):
        errors.append("manifest: file entries must be unique and sorted by path")

    actual_paths = sorted(
        str(path.relative_to(CORPUS))
        for path in CORPUS.rglob("*")
        if path.is_file() and path != MANIFEST
    )
    if actual_paths != sorted(listed_paths):
        errors.append("manifest: file inventory is incomplete or contains an unlisted file")

    for relative, payload in parsed.items():
        if isinstance(payload, dict):
            if payload.get("schema_version") != 1:
                errors.append(f"{relative}: schema_version must be 1")
            if payload.get("corpus_version") != "parity-v1":
                errors.append(f"{relative}: corpus_version must be parity-v1")
        walk_safe_content(payload, relative, errors)
    return parsed, len(entries)


def validate_scenarios(
    payload: Any, lanes: Any, errors: list[str]
) -> tuple[int, int]:
    payload = require_mapping(payload, "scenarios", errors)
    if payload is None:
        return 0, 0
    if payload.get("evidence_class") != "test_definition":
        errors.append("scenarios: evidence_class must be test_definition")
    if payload.get("data_origin") != "deterministic_generated_metadata":
        errors.append("scenarios: data_origin must be deterministic generated metadata")
    if payload.get("oracle_status") != "declared_expected_behavior_not_observed_output":
        errors.append("scenarios: oracle status must not claim observed evidence")

    scenarios = payload.get("scenarios")
    if not isinstance(scenarios, list) or not scenarios:
        errors.append("scenarios: scenarios must be a non-empty list")
        return 0, 0

    identifiers: set[str] = set()
    fast_ids: set[str] = set()
    full_ids: set[str] = set()
    coverage: dict[str, set[str]] = {flow: set() for flow in REQUIRED_FLOWS}
    for index, scenario in enumerate(scenarios):
        label = f"scenarios.scenarios[{index}]"
        if not isinstance(scenario, dict):
            errors.append(f"{label}: expected an object")
            continue
        identifier = scenario.get("id")
        if not isinstance(identifier, str) or not re.fullmatch(r"[a-z0-9-]{8,96}", identifier):
            errors.append(f"{label}: invalid id")
            continue
        if identifier in identifiers:
            errors.append(f"{label}: duplicate id")
        identifiers.add(identifier)

        flow = scenario.get("flow")
        scenario_class = scenario.get("scenario_class")
        lane = scenario.get("lane")
        assertions = scenario.get("assertions")
        if flow not in REQUIRED_FLOWS:
            errors.append(f"{label}: unknown charter-critical flow")
        if scenario_class not in REQUIRED_SCENARIO_CLASSES:
            errors.append(f"{label}: scenario_class must be happy or failure")
        if not isinstance(assertions, list) or not assertions or not all(
            isinstance(assertion, str) and assertion for assertion in assertions
        ):
            errors.append(f"{label}: non-empty assertions are required")

        if lane == "fast":
            fast_ids.add(identifier)
            if scenario.get("execution_context") != "local_simulated":
                errors.append(f"{label}: fast scenarios must be local simulated")
            if flow in coverage and scenario_class in REQUIRED_SCENARIO_CLASSES:
                coverage[flow].add(scenario_class)
            cap_expected = scenario.get("cap_expected")
            frame_expected = scenario.get("frame_expected")
            if cap_expected != frame_expected:
                errors.append(f"{label}: fast parity oracle must initially match")
            if not isinstance(cap_expected, dict) or not isinstance(cap_expected.get("status"), int):
                errors.append(f"{label}: fast expected status is required")
            elif not 100 <= cap_expected["status"] <= 599:
                errors.append(f"{label}: fast expected status is outside HTTP bounds")
        elif lane == "full":
            full_ids.add(identifier)
            if scenario.get("execution_context") not in {
                "protected_hardware_required",
                "protected_provider_required",
            }:
                errors.append(f"{label}: full scenario must name a protected context")
            for expected_name in ("cap_expected", "frame_expected"):
                expected = scenario.get(expected_name)
                if expected != {"status": 0, "state": "context_required"}:
                    errors.append(f"{label}: placeholders cannot claim an observed result")
        else:
            errors.append(f"{label}: lane must be fast or full")

    for flow, classes in sorted(coverage.items()):
        if classes != REQUIRED_SCENARIO_CLASSES:
            errors.append(f"scenarios: fast lane lacks happy/failure pair for {flow}")

    if isinstance(lanes, dict):
        fast_lane = lanes.get("fast", {})
        full_lane = lanes.get("full", {})
        declared_fast = fast_lane.get("scenario_ids") if isinstance(fast_lane, dict) else None
        declared_full = full_lane.get("scenario_ids") if isinstance(full_lane, dict) else None
        if string_set(declared_fast) != fast_ids:
            errors.append("manifest: fast scenario_ids do not match scenario definitions")
        if string_set(declared_full) != full_ids:
            errors.append("manifest: full scenario_ids do not match scenario definitions")
    return len(fast_ids), len(full_ids)


def valid_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def almost_equal(left: float, right: float) -> bool:
    return math.isclose(left, right, rel_tol=1e-12, abs_tol=1e-12)


def string_set(value: Any) -> set[str] | None:
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        return None
    result = set(value)
    if len(result) != len(value):
        return None
    return result


def validate_baselines(payload: Any, errors: list[str]) -> int:
    payload = require_mapping(payload, "baselines", errors)
    if payload is None:
        return 0
    if payload.get("evidence_class") != "local_simulated":
        errors.append("baselines: evidence_class must be local_simulated")
    if payload.get("status") != "deterministic_sample_not_release_measurement":
        errors.append("baselines: status must disclaim release measurement")
    generator = payload.get("generator")
    if not isinstance(generator, dict) or not isinstance(generator.get("seed"), str):
        errors.append("baselines: deterministic generator and seed are required")

    baselines = payload.get("baselines")
    if not isinstance(baselines, list) or len(baselines) < 4:
        errors.append("baselines: at least four deterministic metrics are required")
        return 0
    metric_names: set[str] = set()
    for index, baseline in enumerate(baselines):
        label = f"baselines.baselines[{index}]"
        if not isinstance(baseline, dict):
            errors.append(f"{label}: expected an object")
            continue
        metric = baseline.get("metric")
        if not isinstance(metric, str) or not metric:
            errors.append(f"{label}: metric is required")
        elif metric in metric_names:
            errors.append(f"{label}: duplicate metric")
        else:
            metric_names.add(metric)
        samples = baseline.get("samples")
        repeat_count = baseline.get("repeat_count")
        if (
            not isinstance(samples, list)
            or len(samples) < 5
            or not all(valid_number(value) for value in samples)
            or repeat_count != len(samples)
        ):
            errors.append(f"{label}: repeat_count and at least five finite samples are required")
            continue
        calculated = {
            "mean": statistics.fmean(samples),
            "population_variance": statistics.pvariance(samples),
            "minimum": min(samples),
            "maximum": max(samples),
        }
        for field, expected in calculated.items():
            declared = baseline.get(field)
            if not valid_number(declared) or not almost_equal(float(declared), float(expected)):
                errors.append(f"{label}: {field} does not match deterministic samples")
        budget = baseline.get("comparison_budget")
        if not isinstance(budget, dict) or not budget:
            errors.append(f"{label}: comparison_budget is required")
    return len(baselines)


def validate_diff(payload: Any, label: str, errors: list[str]) -> list[str]:
    payload = require_mapping(payload, label, errors)
    if payload is None:
        return []
    if payload.get("evidence_class") != "local_simulated":
        errors.append(f"{label}: evidence_class must be local_simulated")
    if payload.get("oracle_status") != "declared_expected_behavior_not_observed_output":
        errors.append(f"{label}: oracle status must disclaim observed output")
    comparisons = payload.get("comparisons")
    if not isinstance(comparisons, list) or not comparisons:
        errors.append(f"{label}: comparisons must be a non-empty list")
        return []

    mismatches: list[str] = []
    paths: set[str] = set()
    for index, comparison in enumerate(comparisons):
        item_label = f"{label}.comparisons[{index}]"
        if not isinstance(comparison, dict):
            errors.append(f"{item_label}: expected an object")
            continue
        path = comparison.get("path")
        if not isinstance(path, str) or not path:
            errors.append(f"{item_label}: path is required")
            continue
        if path in paths:
            errors.append(f"{item_label}: duplicate path")
        paths.add(path)
        if comparison.get("strategy") != "exact":
            errors.append(f"{item_label}: only deterministic exact comparison is supported")
            continue
        if comparison.get("cap_value") != comparison.get("frame_value"):
            mismatches.append(path)

    expected_paths = payload.get("expected_mismatch_paths")
    expected_path_set = string_set(expected_paths)
    if expected_path_set is None or expected_paths != sorted(expected_path_set):
        errors.append(f"{label}: expected_mismatch_paths must be unique and sorted")
    elif sorted(mismatches) != expected_paths:
        errors.append(f"{label}: comparator result does not match declared mismatch paths")

    expected_gate = payload.get("expected_gate")
    if expected_gate == "pass":
        if mismatches:
            errors.append(f"{label}: positive comparator control did not pass")
    elif expected_gate == "mismatch":
        if not mismatches:
            errors.append(f"{label}: intentional mismatch gate did not fire")
        if payload.get("fixture_purpose") != "negative_comparator_control_intentional_mismatch":
            errors.append(f"{label}: mismatch fixture is not explicitly marked intentional")
    else:
        errors.append(f"{label}: expected_gate must be pass or mismatch")
    return sorted(mismatches)


def validate_placeholders(
    payload: Any, full_lane: Any, errors: list[str]
) -> tuple[int, set[str]]:
    payload = require_mapping(payload, "protected placeholders", errors)
    if payload is None:
        return 0, set()
    if (
        payload.get("evidence_class") != "protected_placeholder"
        or payload.get("status") != "not_collected"
        or payload.get("is_live_evidence") is not False
        or payload.get("promotion_effect") != "blocks_full_release_gate"
    ):
        errors.append("protected placeholders: root must explicitly block live-evidence claims")
    records = payload.get("records")
    if not isinstance(records, list) or not records:
        errors.append("protected placeholders: records must be a non-empty list")
        return 0, set()

    identifiers: set[str] = set()
    os_families: set[str] = set()
    context_kinds: set[str] = set()
    for index, record in enumerate(records):
        label = f"protected placeholders.records[{index}]"
        if not isinstance(record, dict):
            errors.append(f"{label}: expected an object")
            continue
        identifier = record.get("id")
        if not isinstance(identifier, str) or not identifier:
            errors.append(f"{label}: id is required")
            continue
        if identifier in identifiers:
            errors.append(f"{label}: duplicate id")
        identifiers.add(identifier)
        required_null = ("observed_context", "observed_at", "artifact_sha256", "result")
        if (
            record.get("evidence_class") != "protected_placeholder"
            or record.get("status") != "not_collected"
            or record.get("is_live_evidence") is not False
            or record.get("blocks_lane") != "full"
            or any(record.get(field) is not None for field in required_null)
        ):
            errors.append(f"{label}: placeholder could be mistaken for collected evidence")
        context = record.get("required_context")
        if not isinstance(context, dict):
            errors.append(f"{label}: required_context is required")
            continue
        kind = context.get("kind")
        if isinstance(kind, str):
            context_kinds.add(kind)
        family = context.get("os_family")
        if isinstance(family, str):
            os_families.add(family)
        families = context.get("os_families")
        if isinstance(families, list):
            os_families.update(value for value in families if isinstance(value, str))

    if not {"macos", "windows", "linux"}.issubset(os_families):
        errors.append("protected placeholders: macOS, Windows, and Linux contexts are required")
    if not {"os_hardware", "provider", "provider_and_hardware"}.issubset(context_kinds):
        errors.append("protected placeholders: OS, hardware, and provider contexts are required")
    if isinstance(full_lane, dict):
        declared = full_lane.get("protected_record_ids")
        if string_set(declared) != identifiers:
            errors.append("manifest: full protected_record_ids do not match placeholders")
    return len(records), identifiers


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--require-full",
        action="store_true",
        help="fail while protected provider/hardware evidence is still represented by placeholders",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    errors: list[str] = []
    manifest_payload = load_json(MANIFEST, errors)
    if manifest_payload is None:
        print("parity evidence validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    walk_safe_content(manifest_payload, "manifest", errors)
    manifest = validate_manifest(manifest_payload, errors)
    if manifest is None:
        print("parity evidence validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    parsed, artifact_count = validate_file_inventory(manifest, errors)
    lanes = manifest.get("lanes", {})
    fast_count, full_count = validate_scenarios(
        parsed.get("scenarios.json"), lanes, errors
    )
    baseline_count = validate_baselines(
        parsed.get("baselines/local-simulated.json"), errors
    )
    positive_mismatches = validate_diff(
        parsed.get("diffs/cap-frame-match.json"), "positive diff control", errors
    )
    intentional_mismatches = validate_diff(
        parsed.get("diffs/cap-frame-intentional-mismatch.json"),
        "intentional mismatch control",
        errors,
    )
    full_lane = lanes.get("full", {}) if isinstance(lanes, dict) else {}
    placeholder_count, _ = validate_placeholders(
        parsed.get("protected-context-placeholders.json"), full_lane, errors
    )

    if positive_mismatches:
        errors.append("positive diff control unexpectedly contains mismatches")
    if not intentional_mismatches:
        errors.append("intentional mismatch self-test did not fire")

    if errors:
        print("parity evidence validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(f"immutable corpus: PASS ({artifact_count} hashed JSON artifacts)")
    print(
        "fast lane: PASS "
        f"({fast_count} local simulated scenarios; {baseline_count} deterministic baselines)"
    )
    print(
        "intentional Cap-vs-Frame mismatch gate: FIRED "
        f"({len(intentional_mismatches)} declared mismatches)"
    )
    print(
        "full lane: BLOCKED "
        f"({full_count} protected scenarios; {placeholder_count} records not collected; "
        "no live provider/hardware evidence claimed)"
    )
    if args.require_full:
        print(
            "full parity promotion cannot pass with protected placeholders",
            file=sys.stderr,
        )
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
