#!/usr/bin/env python3
"""Validate Issue-29 conformance fixtures and emit bounded local evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import NoReturn


ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / "fixtures" / "media-conformance" / "v1"
MAX_JSON_BYTES = 2_000_000
SAFE_ID = re.compile(r"^[a-z][a-z0-9_-]{0,127}$")
SAFE_TOKEN = re.compile(r"^[a-z0-9][a-z0-9_-]{0,127}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")

EXPECTED_DIMENSIONS = {
    "platforms": {"macos_arm64_release", "windows_x86_64_release", "linux_x86_64_preview"},
    "sources": {"display", "window", "region", "camera", "microphone", "system_audio", "generated_file", "edit_timeline", "recording_segments"},
    "modes": {"instant_recording", "studio_recording", "managed_derivative", "native_derivative", "hybrid_fallback"},
    "video_codecs": {"vp8", "h264", "h265", "vp9", "av1", "none", "unsupported"},
    "audio_codecs": {"opus", "aac", "mp3", "pcm", "none", "unsupported"},
    "containers": {"webm", "mp4", "quicktime", "matroska", "wave", "unsupported"},
    "resolutions": {"320x180", "1920x1080", "2000x2000", "2001x2000", "7680x4320"},
    "executors": {"native_gstreamer", "cloudflare_media", "external_provider", "control_plane"},
    "capability_boundaries": {
        "input_bytes_99999999", "input_bytes_100000000",
        "input_duration_ms_600000", "input_duration_ms_600001",
        "output_dimension_10", "output_dimension_9", "output_dimension_2000", "output_dimension_2001",
        "output_duration_ms_1000", "output_duration_ms_999", "output_duration_ms_60000", "output_duration_ms_60001",
        "quota_available", "quota_exhausted", "timeout_within_budget", "timeout_over_budget",
    },
    "faults": {
        "device_lost", "permission_denied", "process_crash", "disk_full", "network_lost",
        "provider_error", "provider_outage", "provider_quota", "managed_output_drift",
        "cancellation", "unsupported_codec", "hardware_encoder_failure", "timeout",
    },
}
CASE_KEYS = {
    "id", "lane", "platform", "source", "mode", "video_codec", "audio_codec",
    "container", "resolution", "executor", "scenario", "baseline", "budget",
    "result_contract", "evidence_scope",
}
EXPECTED_REGRESSIONS = {
    "seed_quality": "must_fail_perceptual_gate",
    "seed_sync": "must_fail_av_sync_gate",
    "seed_recovery": "must_fail_recovery_state_gate",
    "seed_resource": "must_fail_resource_trend_gate",
    "seed_routing": "must_fail_executor_routing_gate",
    "seed_provider_limit": "must_fail_provider_preflight_gate",
    "seed_managed_output_drift": "must_fail_output_compatibility_gate",
    "seed_fallback": "must_fail_fallback_policy_gate",
    "seed_repeat_cost": "must_fail_cost_idempotency_gate",
}
EXPECTED_PROTECTED = {
    "native-macos-arm64-capture-and-soak",
    "native-windows-x86-64-capture-and-soak",
    "native-linux-x86-64-preview-capture-and-soak",
    "managed-cloudflare-media-remote",
    "cross-executor-generated-media-conformance",
    "external-provider-adapter-conformance",
    "seeded-fuzz-and-crash-triage",
}
EXPECTED_MATRIX_LANES = {"offline", "native_hardware", "remote_managed", "cross_executor", "external_provider"}
EXPECTED_PROTECTED_LANES = {"native_hardware", "remote_managed", "cross_executor", "external_provider", "scheduled_fuzz"}


class ContractError(RuntimeError):
    pass


def fail(message: str) -> NoReturn:
    raise ContractError(message)


def load_json(name: str) -> tuple[dict[str, object], bytes]:
    path = FIXTURES / name
    if path.is_symlink() or not path.is_file():
        fail(f"missing or symlinked fixture: {name}")
    raw = path.read_bytes()
    if not raw or len(raw) > MAX_JSON_BYTES or b"\x00" in raw:
        fail(f"unsafe fixture size or encoding: {name}")
    try:
        value = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid JSON in {name}: {error}")
    if not isinstance(value, dict):
        fail(f"top level must be an object: {name}")
    return value, raw


def exact_keys(value: dict[str, object], expected: set[str], label: str) -> None:
    actual = set(value)
    if actual != expected:
        fail(f"{label} key drift: missing={sorted(expected - actual)} extra={sorted(actual - expected)}")


def safe_id(value: object, label: str) -> str:
    if not isinstance(value, str) or SAFE_ID.fullmatch(value) is None:
        fail(f"unsafe {label}")
    return value


def safe_token(value: object, label: str) -> str:
    if not isinstance(value, str) or SAFE_TOKEN.fullmatch(value) is None:
        fail(f"unsafe {label}")
    return value


def validate_matrix() -> tuple[dict[str, object], str, int]:
    matrix, raw = load_json("matrix.json")
    exact_keys(matrix, {"schema_version", "matrix_version", "charter_revision", "cap_reference_commit", "managed_contract_revision", "dimensions", "budgets", "cases"}, "matrix")
    if matrix["schema_version"] != 1 or matrix["matrix_version"] != "media-conformance-v1":
        fail("unsupported matrix version")
    if matrix["cap_reference_commit"] != "6ba69561ac86b8efdb17616d6727f9638015546b":
        fail("Cap reference drifted")
    if matrix["managed_contract_revision"] != "cloudflare-media-binding-2026-06-10":
        fail("managed contract revision drifted")

    dimensions = matrix["dimensions"]
    if not isinstance(dimensions, dict):
        fail("dimensions must be an object")
    exact_keys(dimensions, set(EXPECTED_DIMENSIONS), "dimensions")
    for name, expected in EXPECTED_DIMENSIONS.items():
        values = dimensions[name]
        if not isinstance(values, list) or len(values) != len(set(values)) or set(values) != expected:
            fail(f"dimension {name} drifted")

    budgets = matrix["budgets"]
    if not isinstance(budgets, dict):
        fail("budgets must be an object")
    exact_keys(budgets, {"charter_release", "offline_detector_sensitivity", "protected_cost"}, "budgets")
    charter = budgets["charter_release"]
    if charter != {
        "av_start_offset_absolute_ms": 80,
        "av_drift_absolute_ms_after_60_minutes": 50,
        "recording_60s_1080p_time_to_share_p95_ms": 30000,
        "public_playback_start_p95_ms": 2000,
        "capacity_headroom_basis_points": 3000,
    }:
        fail("charter release budgets drifted")
    protected_cost = budgets["protected_cost"]
    if not isinstance(protected_cost, dict) or protected_cost.get("maximum_microunits") is not None or protected_cost.get("concurrency") != 1:
        fail("remote cost must remain unapproved and concurrency one")

    cases = matrix["cases"]
    if not isinstance(cases, list) or len(cases) < 40:
        fail("matrix must contain at least 40 bounded cases")
    seen: set[str] = set()
    covered: dict[str, set[str]] = {name: set() for name in EXPECTED_DIMENSIONS}
    regression_results: dict[str, str] = {}
    covered_lanes: set[str] = set()
    for index, item in enumerate(cases):
        if not isinstance(item, dict):
            fail(f"matrix case {index} is not an object")
        exact_keys(item, CASE_KEYS, f"matrix case {index}")
        case_id = safe_id(item["id"], f"case {index} ID")
        if case_id in seen:
            fail(f"duplicate matrix case: {case_id}")
        seen.add(case_id)
        for field in ("lane", "platform", "source", "mode", "video_codec", "audio_codec", "container", "resolution", "executor", "scenario", "baseline", "budget", "result_contract", "evidence_scope"):
            safe_token(item[field], f"{case_id}.{field}")
        for dimension, field in (
            ("platforms", "platform"), ("sources", "source"), ("modes", "mode"),
            ("video_codecs", "video_codec"), ("audio_codecs", "audio_codec"),
            ("containers", "container"), ("resolutions", "resolution"), ("executors", "executor"),
        ):
            value = str(item[field])
            if value not in EXPECTED_DIMENSIONS[dimension]:
                fail(f"{case_id} uses undeclared {dimension} value")
            covered[dimension].add(value)
        scenario = str(item["scenario"])
        if scenario in EXPECTED_DIMENSIONS["capability_boundaries"]:
            covered["capability_boundaries"].add(scenario)
        if scenario in EXPECTED_DIMENSIONS["faults"]:
            covered["faults"].add(scenario)
        if scenario in EXPECTED_REGRESSIONS:
            regression_results[scenario] = str(item["result_contract"])
        lane = item["lane"]
        if lane not in EXPECTED_MATRIX_LANES:
            fail(f"matrix case {case_id} uses undeclared lane")
        covered_lanes.add(str(lane))
        scope = item["evidence_scope"]
        if lane == "offline" and scope == "protected_not_collected":
            fail(f"offline case {case_id} claims protected scope")
        if lane != "offline" and scope != "protected_not_collected":
            fail(f"protected case {case_id} lacks the not-collected boundary")
    for dimension, expected in EXPECTED_DIMENSIONS.items():
        if covered[dimension] != expected:
            fail(f"matrix cases do not cover {dimension}: missing={sorted(expected - covered[dimension])}")
    if regression_results != EXPECTED_REGRESSIONS:
        fail("seeded regression-to-gate mapping drifted")
    if covered_lanes != EXPECTED_MATRIX_LANES:
        fail(f"matrix lane coverage drifted: missing={sorted(EXPECTED_MATRIX_LANES - covered_lanes)}")
    return matrix, hashlib.sha256(raw).hexdigest(), len(cases)


def validate_offline() -> tuple[dict[str, object], str]:
    suite, raw = load_json("offline-cases.json")
    exact_keys(suite, {"schema_version", "suite_id", "seed", "media_pairs", "progress_traces", "resource_traces", "latency_distributions", "fault_matrix", "routing_observations", "logical_results", "regression_gates"}, "offline suite")
    if suite["schema_version"] != 1 or suite["suite_id"] != "media-conformance-offline-v1" or suite["seed"] != "frame-media-conformance-20260716":
        fail("offline suite identity drifted")
    minimums = {
        "media_pairs": 3, "progress_traces": 3, "resource_traces": 2,
        "latency_distributions": 2, "fault_matrix": 13,
        "routing_observations": 4, "logical_results": 2, "regression_gates": 9,
    }
    for field, minimum in minimums.items():
        value = suite[field]
        if not isinstance(value, list) or len(value) < minimum:
            fail(f"offline suite lacks {field}")
    regression_rows = suite["regression_gates"]
    expected_pairs = {
        ("quality", "perceptual"), ("sync", "av_sync"), ("recovery", "recovery_state"),
        ("resource", "resource_trend"), ("routing", "executor_routing"),
        ("provider_limit", "provider_preflight"),
        ("managed_output_drift", "output_compatibility"),
        ("fallback", "fallback_policy"), ("repeat_cost", "cost_idempotency"),
    }
    if {tuple(row) for row in regression_rows if isinstance(row, list)} != expected_pairs:
        fail("offline regression corpus is incomplete")
    return suite, hashlib.sha256(raw).hexdigest()


def validate_protected(require_protected: bool) -> tuple[dict[str, object], str, int]:
    plan, raw = load_json("protected-lanes.json")
    exact_keys(plan, {"schema_version", "plan_id", "admission_policy", "records"}, "protected plan")
    if plan["schema_version"] != 1 or plan["plan_id"] != "media-conformance-protected-v1":
        fail("protected plan identity drifted")
    admission = plan["admission_policy"]
    if not isinstance(admission, dict):
        fail("protected admission policy must be an object")
    required_true = {"synthetic_inputs_only", "isolated_namespace_required", "scoped_credentials_required", "hard_timeout_required", "cleanup_exact_prefix_only", "cost_approval_required"}
    if any(admission.get(field) is not True for field in required_true) or admission.get("concurrency") != 1:
        fail("protected admission controls weakened")
    if admission.get("waiver_required_fields") != ["owner", "user_impact", "expiry", "cost_security_review", "rollback"]:
        fail("waiver schema drifted")
    records = plan["records"]
    if not isinstance(records, list) or len(records) != len(EXPECTED_PROTECTED):
        fail("protected record inventory drifted")
    ids: set[str] = set()
    lanes: set[str] = set()
    pending = 0
    for record in records:
        if not isinstance(record, dict):
            fail("protected record is not an object")
        record_id = safe_id(record.get("id"), "protected record ID")
        ids.add(record_id)
        lane = safe_id(record.get("lane"), f"protected record {record_id} lane")
        lanes.add(lane)
        if record.get("status") == "not_collected":
            pending += 1
            if record.get("evidence_sha256") is not None or record.get("observed_at") is not None:
                fail(f"pending record {record_id} carries fabricated evidence")
        elif not isinstance(record.get("evidence_sha256"), str) or SHA256.fullmatch(str(record["evidence_sha256"])) is None:
            fail(f"collected record {record_id} lacks an evidence digest")
        if not isinstance(record.get("required_cases"), list) or not record["required_cases"]:
            fail(f"protected record {record_id} has no required cases")
        if record_id.startswith("native-") and int(record.get("minimum_soak_seconds", 0)) < 3600:
            fail(f"native record {record_id} weakened the one-hour soak")
    if ids != EXPECTED_PROTECTED:
        fail("protected record IDs drifted")
    if lanes != EXPECTED_PROTECTED_LANES:
        fail("protected lane inventory drifted")
    if require_protected and pending:
        fail(f"full promotion blocked: {pending} protected media records are not collected")
    return plan, hashlib.sha256(raw).hexdigest(), pending


def validate_fuzz() -> tuple[dict[str, object], str]:
    corpus, raw = load_json("fuzz-corpus.json")
    exact_keys(corpus, {"schema_version", "corpus_id", "deterministic_seed", "cases", "triage"}, "fuzz corpus")
    if corpus["schema_version"] != 1 or corpus["corpus_id"] != "media-conformance-fuzz-v1" or corpus["deterministic_seed"] != 290429:
        fail("fuzz corpus identity drifted")
    cases = corpus["cases"]
    if not isinstance(cases, list) or len(cases) < 10:
        fail("fuzz corpus is too small")
    ids: set[str] = set()
    for case in cases:
        if not isinstance(case, dict) or set(case) != {"id", "hex", "expected"}:
            fail("invalid fuzz case shape")
        case_id = safe_id(case["id"], "fuzz case ID")
        if case_id in ids:
            fail(f"duplicate fuzz case {case_id}")
        ids.add(case_id)
        encoded = case["hex"]
        if not isinstance(encoded, str) or len(encoded) % 2 or re.fullmatch(r"[0-9a-f]*", encoded) is None:
            fail(f"fuzz case {case_id} has invalid hex")
    triage = corpus["triage"]
    if not isinstance(triage, dict) or triage.get("customer_or_private_bytes_forbidden") is not True:
        fail("fuzz triage privacy boundary weakened")
    return corpus, hashlib.sha256(raw).hexdigest()


def validate_manifest(digests: dict[str, str]) -> str:
    manifest, raw = load_json("manifest.json")
    exact_keys(manifest, {"schema_version", "manifest_id", "immutable", "privacy", "files"}, "manifest")
    if manifest["schema_version"] != 1 or manifest["manifest_id"] != "media-conformance-fixtures-v1" or manifest["immutable"] is not True:
        fail("fixture manifest identity drifted")
    privacy = manifest["privacy"]
    if privacy != {"synthetic_only": True, "contains_customer_data": False, "contains_secrets": False, "contains_media_bytes": False}:
        fail("fixture privacy declaration drifted")
    files = manifest["files"]
    if not isinstance(files, list) or len(files) != len(digests):
        fail("manifest file inventory drifted")
    observed: dict[str, str] = {}
    for item in files:
        if not isinstance(item, dict) or set(item) != {"path", "sha256"}:
            fail("invalid manifest file record")
        path, digest = item["path"], item["sha256"]
        if not isinstance(path, str) or not isinstance(digest, str) or SHA256.fullmatch(digest) is None:
            fail("invalid manifest path or digest")
        observed[path] = digest
    if observed != digests:
        fail("fixture digest mismatch; create a new version instead of mutating consumed evidence")
    return hashlib.sha256(raw).hexdigest()


def validate_implementation() -> None:
    source = (ROOT / "crates" / "media" / "src" / "conformance.rs").read_text(encoding="utf-8")
    contract = (ROOT / "crates" / "media" / "tests" / "conformance_contract.rs").read_text(encoding="utf-8")
    for marker in (
        "evaluate_latency_distribution", "verify_single_logical_result",
        "verify_routing_observation", "verify_seeded_regressions",
        "temperature_milli_celsius", "provider_revision",
    ):
        if marker not in source:
            fail(f"conformance implementation missing {marker}")
    for marker in ("managed_boundaries_route_exactly", "immutable_offline_corpus", "protected_records_remain_uncollected"):
        if marker not in contract:
            fail(f"executable conformance contract missing {marker}")


def safe_output(path: Path) -> Path:
    resolved = path.resolve(strict=False)
    evidence_root = (ROOT / "target" / "evidence").resolve(strict=False)
    if resolved == evidence_root or evidence_root not in resolved.parents:
        fail("evidence output must stay below target/evidence")
    cursor = resolved.parent
    while cursor != evidence_root.parent:
        if cursor.exists() and cursor.is_symlink():
            fail("refusing symlinked evidence output parent")
        if cursor == evidence_root:
            break
        cursor = cursor.parent
    resolved.parent.mkdir(parents=True, exist_ok=True)
    return resolved


def write_json(path: Path, value: dict[str, object]) -> None:
    output = safe_output(path)
    output.write_text(json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--dashboard", type=Path)
    parser.add_argument("--require-protected", action="store_true")
    arguments = parser.parse_args()
    try:
        matrix, matrix_hash, case_count = validate_matrix()
        _, offline_hash = validate_offline()
        plan, protected_hash, pending = validate_protected(arguments.require_protected)
        _, fuzz_hash = validate_fuzz()
        digests = {
            "fuzz-corpus.json": fuzz_hash,
            "matrix.json": matrix_hash,
            "offline-cases.json": offline_hash,
            "protected-lanes.json": protected_hash,
        }
        manifest_hash = validate_manifest(digests)
        validate_implementation()

        evidence: dict[str, object] = {
            "schema_version": 1,
            "suite_id": "media-conformance-offline-v1",
            "evidence_class": "local_definition_and_executable_contract",
            "fixture_manifest_sha256": manifest_hash,
            "matrix_case_count": case_count,
            "protected_pending_count": pending,
            "remote_or_hardware_execution_claimed": False,
            "result": "validated",
            "executable_test": "cargo test --locked -p frame-media --test conformance_contract",
        }
        if arguments.evidence:
            write_json(arguments.evidence, evidence)

        records = plan["records"]
        assert isinstance(records, list)
        rows: list[dict[str, object]] = []
        cases = matrix["cases"]
        assert isinstance(cases, list)
        for case in cases:
            assert isinstance(case, dict)
            if case["lane"] == "offline":
                rows.append({
                    "id": case["id"], "executor": case["executor"], "baseline": case["baseline"],
                    "budget": case["budget"], "result": "definition_validated",
                    "trend": "baseline_no_prior_runs", "flake": "deterministic_no_retry",
                    "cost_microunits": 0, "usage": "local_synthetic_only",
                    "evidence_link": "target/evidence/media-conformance-offline.json",
                })
        for record in records:
            assert isinstance(record, dict)
            rows.append({
                "id": record["id"], "executor": record["lane"], "baseline": "protected_record_required",
                "budget": "charter_and_named_cost_approval", "result": record["status"],
                "trend": "not_available", "flake": "not_measured",
                "cost_microunits": record["cost_microunits"], "usage": "not_collected",
                "evidence_link": None,
            })
        dashboard: dict[str, object] = {
            "schema_version": 1,
            "dashboard_id": "media-conformance-release-v1",
            "fixture_manifest_sha256": manifest_hash,
            "promotion_ready": pending == 0,
            "rows": rows,
        }
        if arguments.dashboard:
            write_json(arguments.dashboard, dashboard)
    except (ContractError, OSError) as error:
        print(f"media conformance contract invalid: {error}", file=sys.stderr)
        return 1
    print(f"media conformance validated: {case_count} matrix cases, {pending} protected records pending")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
