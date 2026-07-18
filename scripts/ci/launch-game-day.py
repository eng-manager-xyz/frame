#!/usr/bin/env python3
"""Run the deterministic, provider-free Issue-44 launch and rollback game."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import pathlib
import re
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures/launch-observability/v1"
POLICY_PATH = FIXTURE / "launch-policy.json"
GAME_PATH = FIXTURE / "local-game.json"
PROTECTED_PATH = FIXTURE / "protected-evidence.json"
LATEST_MIGRATION = sorted((ROOT / "apps/control-plane/migrations").glob("*.sql"))[-1].name
SAFE_ID = re.compile(r"^[a-z0-9][a-z0-9._-]{0,127}$")
GIT_SHA = re.compile(r"^[0-9a-f]{40}$")
SAFE_RELEASE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
MIGRATION = re.compile(r"^[0-9]{4}_[a-z0-9_]+\.sql$")


class GameError(RuntimeError):
    """The launch game contract or one of its assertions failed."""


def read_object(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise GameError(f"cannot read JSON fixture {path}: {error}") from error
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise GameError(f"{path} must be a schema-version 1 JSON object")
    return value


def canonical(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def digest(value: Any) -> str:
    return hashlib.sha256(canonical(value)).hexdigest()


def exact(actual: set[str], expected: set[str], label: str) -> None:
    if actual != expected:
        raise GameError(
            f"{label} mismatch; missing={sorted(expected - actual)}, extra={sorted(actual - expected)}"
        )


def safe_records(records: Any, label: str) -> list[dict[str, Any]]:
    if not isinstance(records, list) or not records:
        raise GameError(f"{label} must be a non-empty list")
    result: list[dict[str, Any]] = []
    seen: set[str] = set()
    for record in records:
        if not isinstance(record, dict) or SAFE_ID.fullmatch(str(record.get("id", ""))) is None:
            raise GameError(f"{label} contains an invalid record")
        if record["id"] in seen:
            raise GameError(f"{label} contains duplicate ID {record['id']}")
        seen.add(record["id"])
        result.append(record)
    return result


def validate_journeys(policy: dict[str, Any], game: dict[str, Any]) -> list[dict[str, Any]]:
    definitions = safe_records(policy.get("synthetics"), "synthetic definitions")
    journeys = safe_records(game.get("journeys"), "local journeys")
    exact({item["id"] for item in journeys}, {item["id"] for item in definitions}, "journeys")
    expected_by_id = {item["id"]: item for item in definitions}
    rows: list[dict[str, Any]] = []
    for journey in journeys:
        observations = journey.get("observations")
        if not isinstance(observations, list) or not observations:
            raise GameError(f"journey {journey['id']} has no observations")
        actual_steps = [item.get("step") for item in observations if isinstance(item, dict)]
        if actual_steps != expected_by_id[journey["id"]].get("steps"):
            raise GameError(f"journey {journey['id']} does not execute the exact ordered contract")
        for index, observation in enumerate(observations):
            if not isinstance(observation.get("duration_ms"), int) or observation["duration_ms"] < 0:
                raise GameError(f"journey {journey['id']} has invalid bounded timing")
            expected = (
                "portfolio_available"
                if journey["id"] == "portfolio_degradation" and index < len(observations) - 1
                else "ok"
            )
            if observation.get("result") != expected:
                raise GameError(f"journey {journey['id']} failed at {observation.get('step')}")
        rows.append(
            {
                "id": journey["id"],
                "steps": len(observations),
                "maximum_step_duration_ms": max(item["duration_ms"] for item in observations),
                "passed": True,
            }
        )
    return sorted(rows, key=lambda item: item["id"])


def validate_failures(policy: dict[str, Any], game: dict[str, Any]) -> list[dict[str, Any]]:
    alerts = safe_records(policy.get("alerts"), "alert definitions")
    injections = safe_records(game.get("failure_injections"), "failure injections")
    by_alert = {item["id"]: item for item in alerts}
    expected_boundaries = {item["boundary"] for item in alerts}
    exact({item.get("boundary") for item in injections}, expected_boundaries, "failure boundaries")
    exact({item.get("alert_id") for item in injections}, set(by_alert), "delivered alerts")
    rows: list[dict[str, Any]] = []
    for injection in injections:
        alert = by_alert[injection["alert_id"]]
        if injection.get("boundary") != alert.get("boundary"):
            raise GameError(f"failure {injection['id']} routed to the wrong boundary")
        seed = injection.get("seed_at_ms")
        fired = injection.get("alert_at_ms")
        if not isinstance(seed, int) or not isinstance(fired, int) or seed < 0 or fired < seed:
            raise GameError(f"failure {injection['id']} has invalid logical timing")
        latency = fired - seed
        if latency > alert.get("target_ms", -1):
            raise GameError(f"failure {injection['id']} missed its alert target")
        runbook = alert.get("runbook")
        if not isinstance(runbook, str) or not runbook.startswith("docs/operations/subdomain-launch.md#"):
            raise GameError(f"alert {alert['id']} has no launch runbook target")
        rows.append(
            {
                "id": injection["id"],
                "boundary": injection["boundary"],
                "alert_id": injection["alert_id"],
                "detection_ms": latency,
                "target_ms": alert["target_ms"],
                "passed": True,
            }
        )
    return sorted(rows, key=lambda item: item["id"])


def validate_cache_privacy(game: dict[str, Any]) -> list[dict[str, Any]]:
    incidents = safe_records(game.get("cache_privacy_incidents"), "cache/privacy incidents")
    exact({item["id"] for item in incidents}, {"private_hit", "stale_deletion", "cookie_variance"}, "cache/privacy incidents")
    rows: list[dict[str, Any]] = []
    for incident in incidents:
        if incident.get("observed") == incident.get("expected"):
            raise GameError(f"seeded incident {incident['id']} is not a failure")
        if incident.get("outcome") != "release_blocked":
            raise GameError(f"seeded incident {incident['id']} was counted as availability success")
        if incident.get("alert_id") not in {"edge_route_or_cache_failed", "privacy_failed"}:
            raise GameError(f"seeded incident {incident['id']} has no actionable privacy alert")
        rows.append({"id": incident["id"], "release_blocked": True, "passed": True})
    return sorted(rows, key=lambda item: item["id"])


def validate_release_join(policy: dict[str, Any], game: dict[str, Any]) -> list[dict[str, Any]]:
    cases = safe_records(game.get("release_cases"), "release join cases")
    exact(
        {item["id"] for item in cases},
        {"current_paired", "n_minus_1_consumer", "incompatible_consumer"},
        "release join cases",
    )
    required = set(policy["release_join"]["required_fields"])
    rows: list[dict[str, Any]] = []
    for case in cases:
        missing = required - set(case)
        if missing:
            raise GameError(f"release case {case['id']} misses {sorted(missing)}")
        if GIT_SHA.fullmatch(str(case["source_git_sha"])) is None:
            raise GameError(f"release case {case['id']} has an invalid source SHA")
        for field in ("worker_release", "render_deploy", "portfolio_consumer"):
            if SAFE_RELEASE.fullmatch(str(case[field])) is None:
                raise GameError(f"release case {case['id']} has invalid {field}")
        if MIGRATION.fullmatch(str(case["migration_level"])) is None:
            raise GameError(f"release case {case['id']} has an invalid migration level")
        if case["migration_level"] != LATEST_MIGRATION:
            raise GameError(f"release case {case['id']} does not name the latest migration")
        compatible = case.get("contract_major") == case.get("consumer_contract_major") == 1
        expected = "compatible" if compatible else "NO_GO"
        if case.get("expected") != expected:
            raise GameError(f"release case {case['id']} did not fail closed on contract drift")
        rows.append(
            {
                "id": case["id"],
                "contract_major": case["contract_major"],
                "consumer_contract_major": case["consumer_contract_major"],
                "outcome": expected,
                "passed": True,
            }
        )
    return sorted(rows, key=lambda item: item["id"])


def validate_capacity(policy: dict[str, Any], game: dict[str, Any]) -> list[dict[str, Any]]:
    cases = safe_records(game.get("capacity_cases"), "capacity cases")
    expected = set(policy["capacity"]["dimensions"])
    exact({item["id"] for item in cases}, expected, "capacity dimensions")
    minimum = policy["capacity"]["minimum_headroom_basis_points"]
    rows: list[dict[str, Any]] = []
    for case in cases:
        demand = case.get("demand_units")
        available = case.get("available_units")
        duration = case.get("duration_seconds")
        if not all(isinstance(value, int) and value > 0 for value in (demand, available, duration)):
            raise GameError(f"capacity case {case['id']} has invalid units or duration")
        headroom = (available - demand) * 10_000 // demand
        if headroom < minimum:
            raise GameError(f"capacity case {case['id']} lacks required headroom")
        rows.append({"id": case["id"], "headroom_basis_points": headroom, "passed": True})
    return sorted(rows, key=lambda item: item["id"])


def validate_rollbacks(policy: dict[str, Any], game: dict[str, Any]) -> list[dict[str, Any]]:
    definitions = safe_records(policy.get("rollback_layers"), "rollback definitions")
    games = safe_records(game.get("rollback_games"), "rollback games")
    exact({item["id"] for item in games}, {item["id"] for item in definitions}, "rollback layers")
    by_id = {item["id"]: item for item in definitions}
    rows: list[dict[str, Any]] = []
    for record in games:
        elapsed = record.get("elapsed_ms")
        if not isinstance(elapsed, int) or elapsed < 0 or elapsed > by_id[record["id"]]["maximum_ms"]:
            raise GameError(f"rollback {record['id']} exceeded its target")
        if record.get("verified") is not True:
            raise GameError(f"rollback {record['id']} was not verified")
        if record.get("unrelated_resources_changed") is not False:
            raise GameError(f"rollback {record['id']} changed unrelated resources")
        if record.get("durable_data_deleted") is not False:
            raise GameError(f"rollback {record['id']} deleted durable data")
        scope = record.get("scope")
        if not isinstance(scope, str) or SAFE_ID.fullmatch(scope) is None:
            raise GameError(f"rollback {record['id']} has an unsafe or unbounded scope")
        rows.append(
            {
                "id": record["id"],
                "elapsed_ms": elapsed,
                "target_ms": by_id[record["id"]]["maximum_ms"],
                "unrelated_resources_changed": False,
                "durable_data_deleted": False,
                "passed": True,
            }
        )
    return sorted(rows, key=lambda item: item["id"])


def privacy_audit(value: Any, forbidden_fields: set[str]) -> None:
    def visit(current: Any) -> None:
        if isinstance(current, dict):
            for key, nested in current.items():
                normalized = str(key).lower().replace("-", "_")
                if normalized in forbidden_fields:
                    raise GameError(f"evidence contains forbidden telemetry field {normalized}")
                visit(nested)
        elif isinstance(current, list):
            for nested in current:
                visit(nested)
        elif isinstance(current, str):
            lowered = current.lower()
            markers = ("bearer ", "x-amz-signature", "signed_url=", "cookie:", "authorization:")
            if any(marker in lowered for marker in markers):
                raise GameError("evidence contains a forbidden credential or signed URL marker")

    visit(value)


def run(policy: dict[str, Any], game: dict[str, Any], protected: dict[str, Any]) -> dict[str, Any]:
    if policy.get("local_evidence_authorizes_launch") is not False:
        raise GameError("local launch game must never authorize launch")
    if game.get("evidence_class") != "local_semantic_simulation":
        raise GameError("local game evidence class is invalid")
    if game.get("uses_generated_media_only") is not True or game.get("provider_or_production_access") is not False:
        raise GameError("local game must use generated media and no provider/production access")
    protected_records = safe_records(protected.get("records"), "protected evidence records")
    if protected.get("local_evidence_may_replace_protected_records") is not False:
        raise GameError("local evidence may not replace protected launch evidence")
    if any(item.get("status") != "not_collected" for item in protected_records):
        raise GameError("checked-in protected evidence overclaims collection")

    report: dict[str, Any] = {
        "schema_version": 1,
        "evidence_kind": "frame_subdomain_launch_local_game_v1",
        "policy_digest": digest(policy),
        "game_digest": digest(game),
        "journeys": validate_journeys(policy, game),
        "failure_localization": validate_failures(policy, game),
        "cache_privacy_incidents": validate_cache_privacy(game),
        "release_compatibility": validate_release_join(policy, game),
        "capacity": validate_capacity(policy, game),
        "rollbacks": validate_rollbacks(policy, game),
        "synthetic_media_class": "generated_non_customer_fixture",
        "provider_or_production_evidence": False,
        "protected_records_collected": 0,
        "protected_records_required": len(protected_records),
        "local_contract_passed": True,
        "recommendation": "NO_GO_PROTECTED_EVIDENCE_REQUIRED",
        "authorizes_launch": False,
        "production_authority_changed": False,
    }
    privacy_audit(report, set(policy["telemetry"]["forbidden_fields"]))
    report["evidence_digest"] = digest(report)
    return report


def mutation_self_test(policy: dict[str, Any], game: dict[str, Any], protected: dict[str, Any]) -> list[str]:
    mutations: list[tuple[str, Any]] = []

    missing = copy.deepcopy(game)
    missing["journeys"].pop()
    mutations.append(("missing_journey", missing))

    late = copy.deepcopy(game)
    late["failure_injections"][0]["alert_at_ms"] = 1000000
    mutations.append(("late_alert", late))

    privacy = copy.deepcopy(game)
    privacy["cache_privacy_incidents"][0]["outcome"] = "availability_success"
    mutations.append(("private_hit_as_success", privacy))

    release = copy.deepcopy(game)
    release["release_cases"][2]["expected"] = "compatible"
    mutations.append(("incompatible_release_promoted", release))

    capacity = copy.deepcopy(game)
    capacity["capacity_cases"][0]["available_units"] = 101
    mutations.append(("insufficient_headroom", capacity))

    rollback = copy.deepcopy(game)
    rollback["rollback_games"][0]["durable_data_deleted"] = True
    mutations.append(("rollback_deleted_data", rollback))

    overclaim = copy.deepcopy(protected)
    overclaim["records"][0]["status"] = "collected"

    rejected: list[str] = []
    for name, mutated in mutations:
        try:
            run(policy, mutated, protected)
        except GameError:
            rejected.append(name)
        else:
            raise GameError(f"launch game mutation was not rejected: {name}")
    try:
        run(policy, game, overclaim)
    except GameError:
        rejected.append("protected_evidence_overclaim")
    else:
        raise GameError("launch game mutation was not rejected: protected_evidence_overclaim")

    forbidden = {"schema_version": 1, "authorization": "redacted"}
    try:
        privacy_audit(forbidden, set(policy["telemetry"]["forbidden_fields"]))
    except GameError:
        rejected.append("forbidden_telemetry_field")
    else:
        raise GameError("launch game mutation was not rejected: forbidden_telemetry_field")
    return rejected


def safe_write(path: pathlib.Path, value: dict[str, Any]) -> None:
    if path.is_symlink():
        raise GameError("evidence output may not be a symbolic link")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--policy", type=pathlib.Path, default=POLICY_PATH)
    parser.add_argument("--game", type=pathlib.Path, default=GAME_PATH)
    parser.add_argument("--protected", type=pathlib.Path, default=PROTECTED_PATH)
    parser.add_argument("--evidence", type=pathlib.Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    try:
        policy = read_object(args.policy)
        game = read_object(args.game)
        protected = read_object(args.protected)
        report = run(policy, game, protected)
        rejected = mutation_self_test(policy, game, protected) if args.self_test else []
        if rejected:
            report["rejected_mutations"] = rejected
            report["evidence_digest"] = digest({key: value for key, value in report.items() if key != "evidence_digest"})
        if args.evidence is not None:
            safe_write(args.evidence, report)
        print(
            "launch game passed: 8 journeys, 12 localized failures, 3 cache/privacy incidents, "
            "6 capacity dimensions, 10 reversible layers, protected launch evidence still required"
        )
        return 0
    except (GameError, KeyError, TypeError, ValueError) as error:
        print(f"launch game failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
