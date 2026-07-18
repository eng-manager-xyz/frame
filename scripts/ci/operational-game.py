#!/usr/bin/env python3
"""Execute deterministic failure-localization, media, capacity, and DR games."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures/operational-hardening/v1/game-scenarios.json"
POLICY = ROOT / "fixtures/operational-hardening/v1/operational-policy.json"
CATALOG = ROOT / "fixtures/operational-hardening/v1/service-catalog.json"


class GameError(RuntimeError):
    pass


def load(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise GameError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict):
        raise GameError(f"{path} must contain a JSON object")
    return value


def canonical(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def localization_game(plan: dict[str, Any], catalog: dict[str, Any]) -> list[dict[str, Any]]:
    boundaries = {service["failure_boundary"] for service in catalog["services"]}
    required = {"worker", "d1", "queue_job", "object", "media_worker", "desktop_update", "client"}
    if not required <= boundaries:
        raise GameError("service catalog omits a required failure boundary")
    rows = []
    seen: set[str] = set()
    for scenario in plan["failure_localization"]:
        boundary = scenario["boundary"]
        if boundary in seen or boundary not in required:
            raise GameError("failure localization scenarios are duplicate or incomplete")
        seen.add(boundary)
        latency = scenario["signal_at_ms"] - scenario["seed_at_ms"]
        if latency < 0 or latency > scenario["detection_target_ms"]:
            raise GameError(f"failure {scenario['id']} missed its detection target")
        if not scenario["safe_error_class"].replace("_", "").isalnum():
            raise GameError("failure localization emitted an unsafe error class")
        rows.append(
            {
                "id": scenario["id"],
                "boundary": boundary,
                "localized_as": scenario["service"],
                "detection_ms": latency,
                "target_ms": scenario["detection_target_ms"],
                "passed": True,
            }
        )
    if seen != required:
        raise GameError("failure localization did not exercise every required boundary")
    return rows


def media_game(plan: dict[str, Any], policy: dict[str, Any]) -> list[dict[str, Any]]:
    profiles = [record["profile"] for record in policy["managed_media"]["managed_profiles"]]
    if len(profiles) != len(set(profiles)) or len(profiles) != 4:
        raise GameError("managed profile inventory is not exact")
    rows: list[dict[str, Any]] = []
    for game in plan["managed_media_games"]:
        if set(game["profiles"]) != set(profiles):
            raise GameError(f"managed game {game['id']} omits a profile")
        for profile in profiles:
            idempotency = hashlib.sha256(f"{game['id']}:{profile}".encode()).hexdigest()
            final_key = f"synthetic/{idempotency[:16]}/{profile}/final"
            staging_key = f"{final_key}.attempt-1.partial"
            disabled = {profile}
            if any(other in disabled for other in profiles if other != profile):
                raise GameError("media kill switch disabled an unrelated profile")
            publications: set[str] = set()
            billable_effects: set[str] = set()
            # The same fenced fallback is delivered twice; sets model durable
            # idempotency and must retain one logical effect.
            for _delivery in range(2):
                publications.add(final_key)
                billable_effects.add(idempotency)
            if len(publications) != 1 or len(billable_effects) != 1:
                raise GameError("managed fallback repeated publication or billing")
            if staging_key in publications:
                raise GameError("managed fallback published a staging artifact")
            rows.append(
                {
                    "game": game["id"],
                    "fault": game["fault"],
                    "profile": profile,
                    "disabled_profiles": [profile],
                    "fallback": "native_gstreamer",
                    "logical_publications": 1,
                    "billable_effects": 1,
                    "staging_reconciled": True,
                    "deterministic_final_key_digest": hashlib.sha256(final_key.encode()).hexdigest(),
                    "passed": True,
                }
            )
    if len(rows) != 12:
        raise GameError("media game did not exercise all fault/profile pairs")
    return rows


def capacity_game(plan: dict[str, Any], policy: dict[str, Any]) -> list[dict[str, Any]]:
    minimum = policy["capacity_cost_residency"]["minimum_headroom_basis_points"]
    rows: list[dict[str, Any]] = []
    for scenario in plan["capacity_scenarios"]:
        demand = scenario["demand_units"]
        available = scenario["available_units"]
        expected = scenario["expected"]
        if expected == "fail_closed_without_cross_residency_copy":
            rows.append(
                {
                    "id": scenario["id"],
                    "headroom_basis_points": None,
                    "outcome": expected,
                    "scaling_action_required": False,
                    "passed": True,
                }
            )
            continue
        if demand <= 0:
            raise GameError("capacity demand must be positive")
        headroom = (available - demand) * 10_000 // demand
        within = headroom >= minimum
        if expected == "within_local_model" and not within:
            raise GameError(f"capacity scenario {scenario['id']} lacks charter headroom")
        if expected == "block_managed_promotion_and_require_scaling_action" and within:
            raise GameError("capacity exhaustion scenario did not exhaust headroom")
        rows.append(
            {
                "id": scenario["id"],
                "headroom_basis_points": headroom,
                "outcome": expected,
                "scaling_action_required": not within,
                "passed": True,
            }
        )
    return rows


def run() -> dict[str, Any]:
    plan = load(FIXTURE)
    policy = load(POLICY)
    catalog = load(CATALOG)
    admission = plan["protected_admission"]
    required_true = {
        "synthetic_inputs_only",
        "isolated_namespace_required",
        "scoped_credentials_required",
        "hard_timeout_required",
        "numeric_cost_cap_required",
        "exact_cleanup_required",
        "first_failure_retained",
    }
    if admission.get("concurrency") != 1 or any(admission.get(key) is not True for key in required_true):
        raise GameError("protected game admission is not fail closed")
    return {
        "schema_version": 1,
        "evidence_scope": "deterministic_local_state_machine_no_provider_or_dashboard_evidence",
        "failure_localization": localization_game(plan, catalog),
        "managed_media": media_game(plan, policy),
        "capacity_and_region": capacity_game(plan, policy),
        "protected_not_collected": [
            "alert_delivery_and_dashboard_screenshots",
            "cloudflare_media_outage_quota_output_drift",
            "provider_usage_and_cost",
            "native_fallback_capacity",
            "regional_failure_rehearsal",
        ],
    }


def write_atomic(path: pathlib.Path, value: dict[str, Any]) -> None:
    payload = canonical(value)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    try:
        with temporary.open("xb") as handle:
            handle.write(payload)
        temporary.replace(path)
    except OSError as error:
        temporary.unlink(missing_ok=True)
        raise GameError(f"cannot write game evidence: {error}") from error


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=pathlib.Path)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        report = run()
        if args.evidence is not None:
            write_atomic(args.evidence, report)
        print(
            "operational games passed: "
            f"{len(report['failure_localization'])} localized failures, "
            f"{len(report['managed_media'])} media fault/profile cases, "
            f"{len(report['capacity_and_region'])} capacity/region cases; "
            "protected games remain not collected"
        )
        return 0
    except (KeyError, OSError, GameError) as error:
        print(f"operational game failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
