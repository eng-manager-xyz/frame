#!/usr/bin/env python3
"""Evaluate the Issue-35 cutover dashboard without changing authority."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import math
import pathlib
import re
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures/cutover-decommission/v1"
DEFAULT_CONTRACT = FIXTURE / "cutover-policy.json"
DEFAULT_SCENARIOS = FIXTURE / "dashboard-scenarios.json"
SHA256 = re.compile(r"^[0-9a-f]{64}$")
SAFE_ID = re.compile(r"^[a-z0-9][a-z0-9._-]{0,127}$")


class EvaluationError(RuntimeError):
    """The input is not a valid bounded dashboard snapshot."""


def read_object(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvaluationError(f"cannot read JSON object {path}: {error}") from error
    if not isinstance(value, dict):
        raise EvaluationError(f"{path} must contain one JSON object")
    return value


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("utf-8")


def digest(value: Any) -> str:
    return hashlib.sha256(canonical(value)).hexdigest()


def pointer(value: dict[str, Any], path: str) -> tuple[bool, Any]:
    if not path.startswith("/"):
        raise EvaluationError(f"gate path is not an absolute JSON pointer: {path!r}")
    current: Any = value
    for raw in path.split("/")[1:]:
        key = raw.replace("~1", "/").replace("~0", "~")
        if not isinstance(current, dict) or key not in current:
            return False, None
        current = current[key]
    return True, current


def numeric(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def gate_passes(operator: str, actual: Any, threshold: Any) -> bool:
    if operator == "eq":
        return type(actual) is type(threshold) and actual == threshold
    if operator == "lte":
        return numeric(actual) and numeric(threshold) and actual <= threshold
    if operator == "gte":
        return numeric(actual) and numeric(threshold) and actual >= threshold
    if operator == "sha256":
        return isinstance(actual, str) and SHA256.fullmatch(actual) is not None
    raise EvaluationError(f"unsupported dashboard gate operator: {operator!r}")


def validate_contract(contract: dict[str, Any]) -> dict[str, dict[str, Any]]:
    if contract.get("schema_version") != 1 or contract.get("decision_never_changes_authority") is not True:
        raise EvaluationError("cutover contract must be schema v1 and non-mutating")
    gates = contract.get("gate_definitions")
    if not isinstance(gates, list) or not gates:
        raise EvaluationError("cutover contract has no gate definitions")
    by_code: dict[str, dict[str, Any]] = {}
    for gate in gates:
        if not isinstance(gate, dict) or not SAFE_ID.fullmatch(str(gate.get("code", ""))):
            raise EvaluationError("cutover contract contains an invalid gate code")
        code = gate["code"]
        if code in by_code:
            raise EvaluationError(f"duplicate dashboard gate: {code}")
        if gate.get("operator") not in {"eq", "lte", "gte", "sha256"}:
            raise EvaluationError(f"dashboard gate has an invalid operator: {code}")
        if not isinstance(gate.get("group"), str) or not gate["group"]:
            raise EvaluationError(f"dashboard gate has no group: {code}")
        pointer({}, gate.get("path", ""))
        by_code[code] = gate
    return by_code


def validate_snapshot_shape(contract: dict[str, Any], snapshot: dict[str, Any]) -> dict[str, dict[str, Any]]:
    if snapshot.get("schema_version") != 1:
        raise EvaluationError("dashboard snapshot must use schema version 1")
    if not SAFE_ID.fullmatch(str(snapshot.get("snapshot_id", ""))):
        raise EvaluationError("dashboard snapshot_id is invalid")
    if snapshot.get("scenario_kind") not in {"synthetic_local_validation", "protected_shape_validation_only", "production_observation"}:
        raise EvaluationError("dashboard scenario_kind is invalid")
    if not isinstance(snapshot.get("synthetic"), bool):
        raise EvaluationError("dashboard synthetic marker must be boolean")
    if snapshot["synthetic"] and snapshot["scenario_kind"] != "synthetic_local_validation":
        raise EvaluationError("synthetic snapshots must be explicitly labeled local validation")
    if not snapshot["synthetic"] and snapshot["scenario_kind"] == "synthetic_local_validation":
        raise EvaluationError("non-synthetic snapshots cannot use the local validation label")
    if not isinstance(snapshot.get("release_sha"), str) or SHA256.fullmatch(snapshot["release_sha"]) is None:
        raise EvaluationError("dashboard release_sha must be a full lowercase SHA-256-shaped Git identity")
    stages = contract.get("ramp_stages")
    if not isinstance(stages, list):
        raise EvaluationError("cutover contract has no ramp stages")
    by_stage = {item.get("id"): item for item in stages if isinstance(item, dict)}
    if snapshot.get("stage_id") not in by_stage:
        raise EvaluationError("dashboard stage_id is not in the immutable ramp plan")
    for field in ("observed_at_ms", "evaluated_at_ms", "window_start_ms", "window_end_ms"):
        if not isinstance(snapshot.get(field), int) or isinstance(snapshot[field], bool) or snapshot[field] < 0:
            raise EvaluationError(f"dashboard {field} must be a non-negative integer")
    phase_gates = snapshot.get("phase_gates")
    if not isinstance(phase_gates, dict) or set(phase_gates) != {"p0", "p1", "p2", "p3", "p4", "p5"}:
        raise EvaluationError("dashboard must contain the exact P0-P5 phase gate inventory")
    for phase, record in phase_gates.items():
        if not isinstance(record, dict) or not isinstance(record.get("signed"), bool):
            raise EvaluationError(f"dashboard {phase} gate record is invalid")
        evidence_digest = record.get("evidence_digest")
        if record["signed"] and (not isinstance(evidence_digest, str) or SHA256.fullmatch(evidence_digest) is None):
            raise EvaluationError(f"signed dashboard {phase} gate lacks an evidence digest")
        if not record["signed"] and evidence_digest is not None:
            raise EvaluationError(f"unsigned dashboard {phase} gate may not carry an evidence digest")
    return by_stage


def structural_results(contract: dict[str, Any], snapshot: dict[str, Any], stage: dict[str, Any]) -> list[dict[str, Any]]:
    observed = snapshot["observed_at_ms"]
    evaluated = snapshot["evaluated_at_ms"]
    start = snapshot["window_start_ms"]
    end = snapshot["window_end_ms"]
    maximum_age = contract["observation"]["maximum_snapshot_age_ms"]
    minimum_window = stage["minimum_observation_ms"]
    checks = [
        (
            "snapshot_freshness",
            observed <= evaluated and evaluated - observed <= maximum_age,
            "evidence",
            evaluated - observed if evaluated >= observed else None,
            maximum_age,
        ),
        (
            "observation_window_binding",
            start <= end == observed,
            "evidence",
            end - start if start <= end else None,
            "window_end_equals_observed_at",
        ),
        (
            "minimum_observation_window",
            start <= end and end - start >= minimum_window,
            "evidence",
            end - start if start <= end else None,
            minimum_window,
        ),
        (
            "production_evidence_attached",
            snapshot["synthetic"] or snapshot.get("evidence", {}).get("production_evidence_attached") is True,
            "evidence",
            snapshot.get("evidence", {}).get("production_evidence_attached"),
            True,
        ),
    ]
    if stage["id"] == "irreversible_finalize":
        evidence = snapshot.get("evidence", {})
        for code, field in (
            ("rollback_expiry_approved", "rollback_expiry_approved"),
            ("final_reconciliation_attached", "final_reconciliation_attached"),
            ("source_retention_approved", "source_retention_approved"),
            ("legacy_write_denial_proved", "legacy_write_denial_proved"),
            ("post_cutover_monitoring_active", "post_cutover_monitoring_active"),
        ):
            checks.append((code, evidence.get(field) is True, "irreversible", evidence.get(field), True))
    return [
        {"code": code, "group": group, "passed": passed, "actual": actual, "threshold": threshold}
        for code, passed, group, actual, threshold in checks
    ]


def evaluate(contract: dict[str, Any], snapshot: dict[str, Any]) -> dict[str, Any]:
    validate_contract(contract)
    stages = validate_snapshot_shape(contract, snapshot)
    stage = stages[snapshot["stage_id"]]
    results = structural_results(contract, snapshot, stage)
    for gate in contract["gate_definitions"]:
        present, actual = pointer(snapshot, gate["path"])
        passed = present and gate_passes(gate["operator"], actual, gate.get("threshold"))
        results.append(
            {
                "code": gate["code"],
                "group": gate["group"],
                "passed": passed,
                "actual": actual if present else None,
                "threshold": gate.get("threshold") if gate["operator"] != "sha256" else "sha256",
            }
        )
    results.sort(key=lambda item: item["code"])
    failed = [item["code"] for item in results if not item["passed"]]
    groups: dict[str, dict[str, int]] = {}
    for result in results:
        group = groups.setdefault(result["group"], {"passed": 0, "failed": 0})
        group["passed" if result["passed"] else "failed"] += 1
    report: dict[str, Any] = {
        "schema_version": 1,
        "evaluator": "frame-cutover-go-no-go-v1",
        "contract_id": contract["policy_id"],
        "contract_digest": digest(contract),
        "snapshot_id": snapshot["snapshot_id"],
        "snapshot_digest": digest(snapshot),
        "release_sha": snapshot["release_sha"],
        "stage_id": snapshot["stage_id"],
        "evaluated_at_ms": snapshot["evaluated_at_ms"],
        "synthetic": snapshot["synthetic"],
        "recommendation": "GO" if not failed else "NO_GO",
        "failed_codes": failed,
        "groups": {key: groups[key] for key in sorted(groups)},
        "results": results,
        "authorizes_transition": False,
        "production_authority_changed": False,
    }
    report["report_digest"] = digest(report)
    return report


def merge(base: dict[str, Any], patch: dict[str, Any]) -> dict[str, Any]:
    output = copy.deepcopy(base)
    for key, value in patch.items():
        if isinstance(value, dict) and isinstance(output.get(key), dict):
            output[key] = merge(output[key], value)
        else:
            output[key] = copy.deepcopy(value)
    return output


def self_test(contract: dict[str, Any], scenarios: dict[str, Any]) -> dict[str, Any]:
    if scenarios.get("schema_version") != 1 or not isinstance(scenarios.get("base_snapshot"), dict):
        raise EvaluationError("dashboard scenario fixture is invalid")
    cases = scenarios.get("scenarios")
    if not isinstance(cases, list) or len(cases) < 10:
        raise EvaluationError("dashboard scenario fixture must cover at least ten cases")
    seen: set[str] = set()
    records: list[dict[str, Any]] = []
    for case in cases:
        if not isinstance(case, dict) or not SAFE_ID.fullmatch(str(case.get("id", ""))) or case["id"] in seen:
            raise EvaluationError("dashboard scenario IDs must be unique and bounded")
        seen.add(case["id"])
        snapshot = merge(scenarios["base_snapshot"], case.get("patch", {}))
        first = evaluate(contract, snapshot)
        second = evaluate(contract, snapshot)
        if canonical(first) != canonical(second):
            raise EvaluationError(f"dashboard evaluation is not deterministic: {case['id']}")
        expected_decision = case.get("expected_decision")
        expected_failed = sorted(case.get("expected_failed_codes", []))
        if first["recommendation"] != expected_decision or first["failed_codes"] != expected_failed:
            raise EvaluationError(
                f"dashboard scenario {case['id']} mismatch: expected {expected_decision}/{expected_failed}, "
                f"got {first['recommendation']}/{first['failed_codes']}"
            )
        if first["authorizes_transition"] is not False or first["production_authority_changed"] is not False:
            raise EvaluationError(f"dashboard scenario attempted authority: {case['id']}")
        records.append(
            {
                "scenario_id": case["id"],
                "recommendation": first["recommendation"],
                "failed_codes": first["failed_codes"],
                "report_digest": first["report_digest"],
                "synthetic": first["synthetic"],
                "authorizes_transition": False,
                "production_authority_changed": False,
            }
        )
    summary: dict[str, Any] = {
        "schema_version": 1,
        "evidence_kind": "synthetic_local_cutover_evaluator_self_test",
        "contract_id": contract["policy_id"],
        "contract_digest": digest(contract),
        "scenario_set_id": scenarios["scenario_set_id"],
        "scenario_set_digest": digest(scenarios),
        "records": records,
        "production_evidence": False,
        "authorizes_transition": False,
        "production_authority_changed": False,
    }
    summary["evidence_digest"] = digest(summary)
    return summary


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contract", type=pathlib.Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--snapshot", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test == (args.snapshot is not None):
        parser.error("choose exactly one of --self-test or --snapshot")
    return args


def safe_write(path: pathlib.Path, report: dict[str, Any]) -> None:
    if path.is_symlink():
        raise EvaluationError("dashboard output may not be a symbolic link")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    args = arguments()
    try:
        contract = read_object(args.contract)
        if args.self_test:
            summary = self_test(contract, read_object(DEFAULT_SCENARIOS))
            if args.output is not None:
                safe_write(args.output, summary)
            print("cutover go/no-go self-test passed: deterministic GO/NO_GO, stale/failure stops, and no authority mutation")
            return 0
        snapshot = read_object(args.snapshot)
        report = evaluate(contract, snapshot)
        if args.output is not None:
            safe_write(args.output, report)
        else:
            print(json.dumps(report, indent=2, sort_keys=True))
        return 0 if report["recommendation"] == "GO" else 1
    except (EvaluationError, KeyError, TypeError, ValueError) as error:
        print(f"cutover dashboard evaluation failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
