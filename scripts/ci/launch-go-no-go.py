#!/usr/bin/env python3
"""Evaluate a redacted Issue-44 protected launch snapshot without changing state."""

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
FIXTURE = ROOT / "fixtures/launch-observability/v1"
POLICY_PATH = FIXTURE / "launch-policy.json"
PROTECTED_PATH = FIXTURE / "protected-evidence.json"
MAX_SNAPSHOT_BYTES = 1024 * 1024
SAFE_ID = re.compile(r"^[a-z0-9][a-z0-9._-]{0,127}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
GIT_SHA = re.compile(r"^[0-9a-f]{40}$")
SAFE_RELEASE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
MIGRATION = re.compile(r"^[0-9]{4}_[a-z0-9_]+\.sql$")
LATEST_MIGRATION = sorted((ROOT / "apps/control-plane/migrations").glob("*.sql"))[-1].name


class EvaluationError(RuntimeError):
    """A launch snapshot or policy violates the bounded redacted contract."""


def read_object(path: pathlib.Path, *, bounded: bool = False) -> dict[str, Any]:
    try:
        if bounded and path.stat().st_size > MAX_SNAPSHOT_BYTES:
            raise EvaluationError("launch snapshot exceeds the 1 MiB bound")
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvaluationError(f"cannot read JSON object {path}: {error}") from error
    if not isinstance(value, dict):
        raise EvaluationError(f"{path} must contain one JSON object")
    return value


def canonical(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest(value: Any) -> str:
    return hashlib.sha256(canonical(value)).hexdigest()


def exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvaluationError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise EvaluationError(
            f"{label} keys mismatch; missing={sorted(expected - actual)}, extra={sorted(actual - expected)}"
        )
    return value


def exact_ids(records: Any, label: str) -> dict[str, dict[str, Any]]:
    if not isinstance(records, list) or not records:
        raise EvaluationError(f"{label} must be a non-empty list")
    output: dict[str, dict[str, Any]] = {}
    for record in records:
        if not isinstance(record, dict) or SAFE_ID.fullmatch(str(record.get("id", ""))) is None:
            raise EvaluationError(f"{label} contains an invalid ID")
        if record["id"] in output:
            raise EvaluationError(f"{label} contains duplicate ID {record['id']}")
        output[record["id"]] = record
    return output


def audit_snapshot(value: Any, forbidden_fields: set[str]) -> None:
    def visit(current: Any) -> None:
        if isinstance(current, dict):
            for key, nested in current.items():
                normalized = str(key).lower().replace("-", "_")
                if normalized in forbidden_fields:
                    raise EvaluationError(f"snapshot contains forbidden field {normalized}")
                visit(nested)
        elif isinstance(current, list):
            for nested in current:
                visit(nested)
        elif isinstance(current, str):
            lowered = current.lower()
            if any(
                marker in lowered
                for marker in ("bearer ", "x-amz-signature", "signed_url=", "cookie:", "authorization:")
            ):
                raise EvaluationError("snapshot contains a credential or signed-URL marker")

    visit(value)


def result(code: str, passed: bool, group: str) -> dict[str, Any]:
    return {"code": code, "group": group, "passed": bool(passed)}


def is_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)


def evaluate(policy: dict[str, Any], protected: dict[str, Any], snapshot: dict[str, Any]) -> dict[str, Any]:
    if policy.get("schema_version") != 1 or policy.get("local_evidence_authorizes_launch") is not False:
        raise EvaluationError("launch policy must be schema v1 and advisory-only")
    protected_by_id = exact_ids(protected.get("records"), "protected evidence ledger")
    if protected.get("local_evidence_may_replace_protected_records") is not False:
        raise EvaluationError("protected evidence policy is not fail closed")

    exact_keys(
        snapshot,
        {
            "schema_version",
            "snapshot_id",
            "scenario_kind",
            "environment",
            "observed_at_ms",
            "evaluated_at_ms",
            "release",
            "dependencies",
            "defects",
            "slos",
            "dashboards",
            "synthetics",
            "capacity",
            "rollbacks",
            "privacy",
            "portfolio",
            "launch",
            "protected_evidence",
        },
        "launch snapshot",
    )
    if snapshot.get("schema_version") != 1:
        raise EvaluationError("launch snapshot must use schema version 1")
    if SAFE_ID.fullmatch(str(snapshot.get("snapshot_id", ""))) is None:
        raise EvaluationError("launch snapshot ID is invalid")
    if snapshot.get("scenario_kind") not in {"protected_shape_validation_only", "production_observation"}:
        raise EvaluationError("launch snapshot scenario kind is invalid")
    if snapshot.get("scenario_kind") == "production_observation" and snapshot.get("environment") != "production":
        raise EvaluationError("a production observation must name the production environment")
    audit_snapshot(snapshot, set(policy["telemetry"]["forbidden_fields"]))

    checks: list[dict[str, Any]] = []
    observed = snapshot.get("observed_at_ms")
    evaluated = snapshot.get("evaluated_at_ms")
    valid_times = (
        isinstance(observed, int)
        and not isinstance(observed, bool)
        and isinstance(evaluated, int)
        and not isinstance(evaluated, bool)
        and 0 <= observed <= evaluated
    )
    checks.append(result("snapshot_fresh", valid_times and evaluated - observed <= 300000, "evidence"))

    release = exact_keys(
        snapshot["release"],
        {
            "source_git_sha",
            "contract_major",
            "worker_release",
            "render_deploy",
            "migration_level",
            "portfolio_consumer",
            "consumer_contract_major",
            "n_minus_1_compatible",
            "endpoint_or_headers_verified",
        },
        "release join",
    )
    release_shape = (
        GIT_SHA.fullmatch(str(release["source_git_sha"])) is not None
        and SAFE_RELEASE.fullmatch(str(release["worker_release"])) is not None
        and SAFE_RELEASE.fullmatch(str(release["render_deploy"])) is not None
        and SAFE_RELEASE.fullmatch(str(release["portfolio_consumer"])) is not None
        and MIGRATION.fullmatch(str(release["migration_level"])) is not None
    )
    checks.append(result("release_fields_safe_and_complete", release_shape, "release"))
    checks.append(
        result(
            "release_contract_current_and_n_minus_1",
            release.get("contract_major") == release.get("consumer_contract_major") == 1
            and release.get("n_minus_1_compatible") is True,
            "release",
        )
    )
    checks.append(result("release_endpoint_or_headers_verified", release.get("endpoint_or_headers_verified") is True, "release"))

    dependencies = exact_keys(
        snapshot["dependencies"],
        {str(issue) for issue in policy["decision"]["all_p7_dependencies_required"]},
        "P7 dependencies",
    )
    for issue, record_value in dependencies.items():
        record = exact_keys(record_value, {"status", "evidence_digest"}, f"dependency {issue}")
        checks.append(
            result(
                f"dependency_{issue}_approved",
                record.get("status") == "approved" and SHA256.fullmatch(str(record.get("evidence_digest"))) is not None,
                "dependencies",
            )
        )

    defects = exact_keys(snapshot["defects"], {"critical_open", "high_open"}, "defects")
    checks.append(result("critical_defects_closed", defects.get("critical_open") == 0, "defects"))
    checks.append(result("high_defects_closed", defects.get("high_open") == 0, "defects"))

    slo_defs = exact_ids(policy.get("slos"), "SLO definitions")
    slos = exact_keys(snapshot["slos"], set(slo_defs), "SLO observations")
    for slo_id, record_value in slos.items():
        record = exact_keys(record_value, {"passed", "sample_count"}, f"SLO {slo_id}")
        checks.append(
            result(
                f"slo_{slo_id}",
                record.get("passed") is True
                and isinstance(record.get("sample_count"), int)
                and record["sample_count"] > 0,
                "slos",
            )
        )

    dashboard_defs = exact_ids(policy.get("dashboards"), "dashboard definitions")
    alert_defs = exact_ids(policy.get("alerts"), "alert definitions")
    dashboards = exact_keys(snapshot["dashboards"], {"exports", "alerts", "privacy_scan_passed"}, "dashboards")
    exports = dashboards.get("exports")
    checks.append(
        result(
            "dashboard_exports_complete",
            isinstance(exports, list) and set(exports) == set(dashboard_defs) and len(exports) == len(set(exports)),
            "observability",
        )
    )
    alerts = exact_keys(dashboards["alerts"], set(alert_defs), "alert observations")
    for alert_id, record_value in alerts.items():
        record = exact_keys(record_value, {"delivered", "delivery_ms", "runbook_linked"}, f"alert {alert_id}")
        delivery = record.get("delivery_ms")
        checks.append(
            result(
                f"alert_{alert_id}",
                record.get("delivered") is True
                and isinstance(delivery, int)
                and 0 <= delivery <= alert_defs[alert_id]["target_ms"]
                and record.get("runbook_linked") is True,
                "observability",
            )
        )
    checks.append(result("dashboard_privacy_scan", dashboards.get("privacy_scan_passed") is True, "privacy"))

    synthetic_defs = exact_ids(policy.get("synthetics"), "synthetic definitions")
    synthetics = exact_keys(snapshot["synthetics"], set(synthetic_defs), "synthetic history")
    for synthetic_id, record_value in synthetics.items():
        record = exact_keys(
            record_value,
            {"passed", "input_class", "cleanup_complete", "direct_r2_transit"},
            f"synthetic {synthetic_id}",
        )
        input_ok = record.get("input_class") == (
            "generated_media" if synthetic_id == "upload_process" else "no_media"
        )
        transit_ok = record.get("direct_r2_transit") is (synthetic_id == "upload_process")
        checks.append(
            result(
                f"synthetic_{synthetic_id}",
                record.get("passed") is True
                and input_ok
                and record.get("cleanup_complete") is True
                and transit_ok,
                "synthetics",
            )
        )

    capacity = exact_keys(
        snapshot["capacity"],
        {
            "minimum_headroom_basis_points",
            "dimensions",
            "numeric_cost_cap_approved",
            "provider_quotas_approved",
            "scaling_actions_closed",
        },
        "capacity",
    )
    dimensions = exact_keys(capacity["dimensions"], set(policy["capacity"]["dimensions"]), "capacity dimensions")
    checks.append(
        result(
            "capacity_headroom",
            is_number(capacity.get("minimum_headroom_basis_points"))
            and capacity["minimum_headroom_basis_points"] >= policy["capacity"]["minimum_headroom_basis_points"]
            and all(value is True for value in dimensions.values()),
            "capacity_cost",
        )
    )
    checks.append(result("numeric_cost_cap_approved", capacity.get("numeric_cost_cap_approved") is True, "capacity_cost"))
    checks.append(result("provider_quotas_approved", capacity.get("provider_quotas_approved") is True, "capacity_cost"))
    checks.append(result("scaling_actions_closed", capacity.get("scaling_actions_closed") is True, "capacity_cost"))

    rollback_defs = exact_ids(policy.get("rollback_layers"), "rollback definitions")
    rollbacks = exact_keys(snapshot["rollbacks"], set(rollback_defs), "rollback observations")
    for rollback_id, record_value in rollbacks.items():
        record = exact_keys(
            record_value,
            {"passed", "elapsed_ms", "data_preserved", "unrelated_resources_changed"},
            f"rollback {rollback_id}",
        )
        elapsed = record.get("elapsed_ms")
        checks.append(
            result(
                f"rollback_{rollback_id}",
                record.get("passed") is True
                and isinstance(elapsed, int)
                and 0 <= elapsed <= rollback_defs[rollback_id]["maximum_ms"]
                and record.get("data_preserved") is True
                and record.get("unrelated_resources_changed") is False,
                "rollback",
            )
        )

    privacy = exact_keys(
        snapshot["privacy"],
        {"forbidden_findings", "cache_incidents_blocked", "support_bundle_passed", "telemetry_audit_passed"},
        "privacy audit",
    )
    checks.append(result("privacy_forbidden_findings_zero", privacy.get("forbidden_findings") == 0, "privacy"))
    checks.append(
        result(
            "cache_privacy_incidents_blocked",
            isinstance(privacy.get("cache_incidents_blocked"), list)
            and set(privacy["cache_incidents_blocked"]) == {"private_hit", "stale_deletion", "cookie_variance"},
            "privacy",
        )
    )
    checks.append(result("support_bundle_privacy", privacy.get("support_bundle_passed") is True, "privacy"))
    checks.append(result("telemetry_privacy_audit", privacy.get("telemetry_audit_passed") is True, "privacy"))

    portfolio = exact_keys(
        snapshot["portfolio"],
        {
            "faults_exercised",
            "availability",
            "baseline_latency_p95_ms",
            "outage_latency_p95_ms",
        },
        "portfolio baseline",
    )
    faults = portfolio.get("faults_exercised")
    expected_faults = set(policy["portfolio_independence"]["required_faults"])
    baseline = portfolio.get("baseline_latency_p95_ms")
    outage = portfolio.get("outage_latency_p95_ms")
    max_regression = policy["portfolio_independence"]["maximum_latency_regression_basis_points"]
    latency_ok = (
        is_number(baseline)
        and baseline > 0
        and is_number(outage)
        and outage >= 0
        and (outage - baseline) * 10_000 <= baseline * max_regression
    )
    checks.append(
        result(
            "portfolio_independent_during_frame_faults",
            isinstance(faults, list)
            and set(faults) == expected_faults
            and len(faults) == len(set(faults))
            and is_number(portfolio.get("availability"))
            and portfolio["availability"] >= policy["portfolio_independence"]["minimum_availability"]
            and latency_ok,
            "portfolio",
        )
    )

    launch = exact_keys(
        snapshot["launch"],
        {
            "sequence_completed",
            "observation_window_ms",
            "decision_makers_confirmed",
            "p7_dependency_signatures_complete",
            "post_launch_decisions",
        },
        "launch record",
    )
    sequence = launch.get("sequence_completed")
    checks.append(
        result(
            "launch_sequence_complete",
            isinstance(sequence, list)
            and sequence == policy["launch_sequence"],
            "launch",
        )
    )
    checks.append(
        result(
            "observation_window_complete",
            isinstance(launch.get("observation_window_ms"), int)
            and launch["observation_window_ms"] >= 86400000,
            "launch",
        )
    )
    checks.append(result("decision_makers_confirmed", launch.get("decision_makers_confirmed") is True, "launch"))
    checks.append(result("p7_dependency_signatures_complete", launch.get("p7_dependency_signatures_complete") is True, "launch"))
    decisions = exact_keys(launch["post_launch_decisions"], set(policy["post_launch_decisions"]), "post-launch decisions")
    allowed_decisions = {"keep_enabled", "disable", "defer", "retain", "remove"}
    checks.append(
        result(
            "post_launch_decisions_complete",
            all(value in allowed_decisions for value in decisions.values()),
            "launch",
        )
    )

    evidence = exact_keys(snapshot["protected_evidence"], set(protected_by_id), "protected evidence snapshot")
    for evidence_id, record_value in evidence.items():
        record = exact_keys(record_value, {"status", "evidence_digest", "owner_acknowledged"}, f"protected evidence {evidence_id}")
        checks.append(
            result(
                f"protected_{evidence_id}",
                record.get("status") == "approved"
                and SHA256.fullmatch(str(record.get("evidence_digest"))) is not None
                and record.get("owner_acknowledged") is True,
                "protected_evidence",
            )
        )

    checks.sort(key=lambda item: item["code"])
    failed = [item["code"] for item in checks if not item["passed"]]
    groups: dict[str, dict[str, int]] = {}
    for check in checks:
        group = groups.setdefault(check["group"], {"passed": 0, "failed": 0})
        group["passed" if check["passed"] else "failed"] += 1
    report: dict[str, Any] = {
        "schema_version": 1,
        "evaluator": "frame-subdomain-launch-go-no-go-v1",
        "policy_digest": digest(policy),
        "snapshot_id": snapshot["snapshot_id"],
        "snapshot_digest": digest(snapshot),
        "scenario_kind": snapshot["scenario_kind"],
        "recommendation": "GO" if not failed else "NO_GO",
        "failed_codes": failed,
        "groups": {key: groups[key] for key in sorted(groups)},
        "results": checks,
        "authorizes_launch": False,
        "production_authority_changed": False,
        "requires_independent_signature": True,
    }
    report["report_digest"] = digest(report)
    return report


def valid_shape_snapshot(policy: dict[str, Any], protected: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "snapshot_id": "synthetic-shape-pass",
        "scenario_kind": "protected_shape_validation_only",
        "environment": "shape-test",
        "observed_at_ms": 100000,
        "evaluated_at_ms": 100001,
        "release": {
            "source_git_sha": "1" * 40,
            "contract_major": 1,
            "worker_release": "worker-shape-test",
            "render_deploy": "render-shape-test",
            "migration_level": LATEST_MIGRATION,
            "portfolio_consumer": "portfolio-shape-test",
            "consumer_contract_major": 1,
            "n_minus_1_compatible": True,
            "endpoint_or_headers_verified": True,
        },
        "dependencies": {
            str(issue): {"status": "approved", "evidence_digest": str(issue % 10) * 64}
            for issue in policy["decision"]["all_p7_dependencies_required"]
        },
        "defects": {"critical_open": 0, "high_open": 0},
        "slos": {item["id"]: {"passed": True, "sample_count": 1} for item in policy["slos"]},
        "dashboards": {
            "exports": [item["id"] for item in policy["dashboards"]],
            "alerts": {
                item["id"]: {"delivered": True, "delivery_ms": 1, "runbook_linked": True}
                for item in policy["alerts"]
            },
            "privacy_scan_passed": True,
        },
        "synthetics": {
            item["id"]: {
                "passed": True,
                "input_class": "generated_media" if item["id"] == "upload_process" else "no_media",
                "cleanup_complete": True,
                "direct_r2_transit": item["id"] == "upload_process",
            }
            for item in policy["synthetics"]
        },
        "capacity": {
            "minimum_headroom_basis_points": 3000,
            "dimensions": {item: True for item in policy["capacity"]["dimensions"]},
            "numeric_cost_cap_approved": True,
            "provider_quotas_approved": True,
            "scaling_actions_closed": True,
        },
        "rollbacks": {
            item["id"]: {
                "passed": True,
                "elapsed_ms": 1,
                "data_preserved": True,
                "unrelated_resources_changed": False,
            }
            for item in policy["rollback_layers"]
        },
        "privacy": {
            "forbidden_findings": 0,
            "cache_incidents_blocked": ["private_hit", "stale_deletion", "cookie_variance"],
            "support_bundle_passed": True,
            "telemetry_audit_passed": True,
        },
        "portfolio": {
            "faults_exercised": policy["portfolio_independence"]["required_faults"],
            "availability": 0.999,
            "baseline_latency_p95_ms": 100,
            "outage_latency_p95_ms": 105,
        },
        "launch": {
            "sequence_completed": policy["launch_sequence"],
            "observation_window_ms": 86400000,
            "decision_makers_confirmed": True,
            "p7_dependency_signatures_complete": True,
            "post_launch_decisions": {item: "defer" for item in policy["post_launch_decisions"]},
        },
        "protected_evidence": {
            item["id"]: {
                "status": "approved",
                "evidence_digest": "a" * 64,
                "owner_acknowledged": True,
            }
            for item in protected["records"]
        },
    }


def self_test(policy: dict[str, Any], protected: dict[str, Any]) -> list[str]:
    base = valid_shape_snapshot(policy, protected)
    passed = evaluate(policy, protected, base)
    if passed["recommendation"] != "GO" or passed["authorizes_launch"] is not False:
        raise EvaluationError("valid protected shape did not produce an advisory GO")

    cases: list[tuple[str, dict[str, Any], str]] = []
    stale = copy.deepcopy(base)
    stale["evaluated_at_ms"] += 300001
    cases.append(("stale_snapshot", stale, "snapshot_fresh"))
    dependency = copy.deepcopy(base)
    dependency["dependencies"]["34"]["status"] = "not_collected"
    cases.append(("dependency_open", dependency, "dependency_34_approved"))
    defect = copy.deepcopy(base)
    defect["defects"]["high_open"] = 1
    cases.append(("high_defect", defect, "high_defects_closed"))
    slo = copy.deepcopy(base)
    slo["slos"]["api_availability"]["passed"] = False
    cases.append(("slo_failed", slo, "slo_api_availability"))
    alert = copy.deepcopy(base)
    alert["dashboards"]["alerts"]["privacy_failed"]["delivery_ms"] = 30001
    cases.append(("alert_late", alert, "alert_privacy_failed"))
    release = copy.deepcopy(base)
    release["release"]["consumer_contract_major"] = 2
    cases.append(("release_drift", release, "release_contract_current_and_n_minus_1"))
    cost = copy.deepcopy(base)
    cost["capacity"]["numeric_cost_cap_approved"] = False
    cases.append(("cost_unapproved", cost, "numeric_cost_cap_approved"))
    rollback = copy.deepcopy(base)
    rollback["rollbacks"]["worker_route_remove"]["data_preserved"] = False
    cases.append(("rollback_data_loss", rollback, "rollback_worker_route_remove"))
    privacy = copy.deepcopy(base)
    privacy["privacy"]["forbidden_findings"] = 1
    cases.append(("privacy_finding", privacy, "privacy_forbidden_findings_zero"))
    portfolio = copy.deepcopy(base)
    portfolio["portfolio"]["availability"] = 0.9
    cases.append(("portfolio_dependency", portfolio, "portfolio_independent_during_frame_faults"))
    evidence = copy.deepcopy(base)
    first_evidence = protected["records"][0]["id"]
    evidence["protected_evidence"][first_evidence]["status"] = "not_collected"
    cases.append(("protected_missing", evidence, f"protected_{first_evidence}"))
    sequence = copy.deepcopy(base)
    sequence["launch"]["sequence_completed"] = sequence["launch"]["sequence_completed"][:-1]
    cases.append(("sequence_incomplete", sequence, "launch_sequence_complete"))

    rejected: list[str] = []
    for name, snapshot, expected_failure in cases:
        report = evaluate(policy, protected, snapshot)
        if report["recommendation"] != "NO_GO" or expected_failure not in report["failed_codes"]:
            raise EvaluationError(f"launch evaluator did not reject {name} at {expected_failure}")
        if report["authorizes_launch"] is not False or report["production_authority_changed"] is not False:
            raise EvaluationError(f"launch evaluator attempted authority in {name}")
        rejected.append(name)

    unsafe = copy.deepcopy(base)
    unsafe["privacy"]["authorization"] = "redacted"
    try:
        evaluate(policy, protected, unsafe)
    except EvaluationError:
        rejected.append("unexpected_or_forbidden_field")
    else:
        raise EvaluationError("launch evaluator admitted an unexpected forbidden field")
    return rejected


def safe_write(path: pathlib.Path, report: dict[str, Any]) -> None:
    if path.is_symlink():
        raise EvaluationError("launch decision output may not be a symbolic link")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--policy", type=pathlib.Path, default=POLICY_PATH)
    parser.add_argument("--protected", type=pathlib.Path, default=PROTECTED_PATH)
    parser.add_argument("--snapshot", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test == (args.snapshot is not None):
        parser.error("choose exactly one of --self-test or --snapshot")
    try:
        policy = read_object(args.policy)
        protected = read_object(args.protected)
        if args.self_test:
            rejected = self_test(policy, protected)
            report = {
                "schema_version": 1,
                "evidence_kind": "launch_go_no_go_shape_self_test",
                "policy_digest": digest(policy),
                "rejected_cases": rejected,
                "provider_or_production_evidence": False,
                "authorizes_launch": False,
                "production_authority_changed": False,
            }
            report["evidence_digest"] = digest(report)
            if args.output is not None:
                safe_write(args.output, report)
            print(
                "launch go/no-go self-test passed: advisory GO shape plus 13 fail-closed cases; "
                "no production authority changed"
            )
            return 0
        snapshot = read_object(args.snapshot, bounded=True)
        report = evaluate(policy, protected, snapshot)
        if args.output is not None:
            safe_write(args.output, report)
        else:
            print(json.dumps(report, indent=2, sort_keys=True))
        return 0 if report["recommendation"] == "GO" else 1
    except (EvaluationError, KeyError, TypeError, ValueError) as error:
        print(f"launch go/no-go evaluation failed: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
