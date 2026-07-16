#!/usr/bin/env python3
"""Validate the complete local Issue-44 launch, observability, and rollback contract."""

from __future__ import annotations

import json
import pathlib
import stat
import subprocess
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures/launch-observability/v1"


class CheckError(RuntimeError):
    """The checked-in launch contract is incomplete or unsafe."""


def load(name: str) -> dict[str, Any]:
    path = FIXTURE / name
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise CheckError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise CheckError(f"{path} must be a schema-version 1 object")
    return value


def exact(actual: set[Any], expected: set[Any], label: str) -> None:
    if actual != expected:
        raise CheckError(
            f"{label} mismatch; missing={sorted(expected - actual)}, extra={sorted(actual - expected)}"
        )


def records(value: Any, label: str) -> list[dict[str, Any]]:
    if not isinstance(value, list) or not value or not all(isinstance(item, dict) for item in value):
        raise CheckError(f"{label} must be a non-empty object list")
    identifiers = [item.get("id") for item in value]
    if not all(isinstance(item, str) and item for item in identifiers) or len(identifiers) != len(set(identifiers)):
        raise CheckError(f"{label} IDs must be non-empty and unique")
    return value


def validate_policy(policy: dict[str, Any]) -> None:
    if policy.get("canonical_origin") != "https://frame.engmanager.xyz":
        raise CheckError("launch policy canonical origin drifted")
    if policy.get("portfolio_origin") != "https://engmanager.xyz":
        raise CheckError("launch policy portfolio origin drifted")
    if policy.get("production_authority_changed") is not False or policy.get("local_evidence_authorizes_launch") is not False:
        raise CheckError("local launch policy overclaims production authority")
    decision = policy.get("decision", {})
    if decision.get("mode") != "advisory_only" or decision.get("missing_stale_failed_or_unsigned") != "NO_GO":
        raise CheckError("launch decision is not fail closed and advisory-only")
    if decision.get("all_p7_dependencies_required") != [34, 35, 37, 38, 39, 40, 41, 42, 43]:
        raise CheckError("launch decision omits or reorders a P7 dependency")
    if decision.get("critical_or_high_open_allowed") != 0:
        raise CheckError("launch policy permits critical or high defects")

    expected_roles = {
        "repository_owner_role",
        "release_commander_role",
        "incident_commander_role",
        "portfolio_owner_role",
        "edge_operator_role",
        "render_operator_role",
        "worker_operator_role",
        "data_recovery_operator_role",
        "media_operations_owner_role",
        "security_approver_role",
        "cost_approver_role",
        "support_lead_role",
    }
    exact(set(policy.get("owners", {}).values()), expected_roles, "launch owner roles")
    contacts = policy.get("contact_registry", {})
    if contacts.get("repository_contains_personal_contacts") is not False or contacts.get("acknowledgement_status") != "not_collected":
        raise CheckError("contact registry embeds contacts or fabricates acknowledgement")

    telemetry = policy.get("telemetry", {})
    required_forbidden = {
        "media",
        "captions",
        "private_title",
        "email",
        "tenant_id",
        "object_key",
        "cookie",
        "token",
        "signed_url",
        "request_body",
        "response_body",
    }
    if not required_forbidden <= set(telemetry.get("forbidden_fields", [])):
        raise CheckError("launch telemetry does not forbid every required sensitive class")
    if telemetry.get("dashboard_status") != "not_collected":
        raise CheckError("dashboard delivery evidence was fabricated locally")
    exact(
        set(telemetry.get("required_dimensions", [])),
        {"service", "boundary", "release", "environment", "operation", "result_class"},
        "telemetry dimensions",
    )

    slos = records(policy.get("slos"), "launch SLOs")
    expected_slos = {
        "portfolio_link_success",
        "edge_availability",
        "edge_latency",
        "render_landing_availability",
        "render_startup",
        "api_availability",
        "api_latency",
        "upload_finalize_success",
        "processing_time_to_share",
        "public_playback_success",
        "cache_privacy_correctness",
        "auth_privacy_correctness",
        "native_queue_age",
        "release_freshness",
    }
    exact({item["id"] for item in slos}, expected_slos, "launch SLOs")
    by_slo = {item["id"]: item for item in slos}
    for slo_id in (
        "portfolio_link_success",
        "edge_availability",
        "render_landing_availability",
        "api_availability",
        "upload_finalize_success",
        "public_playback_success",
    ):
        if by_slo[slo_id].get("objective") != 0.999 or by_slo[slo_id].get("error_budget_basis_points") != 10:
            raise CheckError(f"SLO {slo_id} does not use the approved 99.9% objective")
    for slo_id in ("cache_privacy_correctness", "auth_privacy_correctness", "release_freshness"):
        if by_slo[slo_id].get("objective") != 1.0 or by_slo[slo_id].get("error_budget_basis_points") != 0:
            raise CheckError(f"zero-tolerance SLO {slo_id} has a nonzero budget")
    maxima = {
        "edge_latency": 750,
        "render_startup": 300000,
        "api_latency": 500,
        "processing_time_to_share": 30000,
        "native_queue_age": 30000,
    }
    for slo_id, maximum in maxima.items():
        if by_slo[slo_id].get("maximum") != maximum:
            raise CheckError(f"SLO {slo_id} threshold drifted")

    services = records(policy.get("service_catalog"), "launch service catalog")
    expected_services = {
        "portfolio",
        "cloudflare_edge",
        "render_web",
        "worker_api",
        "d1_metadata",
        "r2_objects",
        "managed_media",
        "native_media",
        "public_playback",
        "auth_privacy",
        "release_contract",
    }
    exact({item["id"] for item in services}, expected_services, "launch service catalog")
    service_ids = {item["id"] for item in services}
    for service in services:
        if service.get("owner") not in expected_roles:
            raise CheckError(f"launch service {service['id']} has no owner role")
        dependencies = service.get("dependencies")
        if not isinstance(dependencies, list) or not set(dependencies) <= service_ids or service["id"] in dependencies:
            raise CheckError(f"launch service {service['id']} has invalid dependencies")
        if not set(service.get("slo_ids", [])) <= expected_slos or not service.get("slo_ids"):
            raise CheckError(f"launch service {service['id']} has invalid SLO ownership")
        if not str(service.get("runbook", "")).startswith("docs/operations/subdomain-launch.md#"):
            raise CheckError(f"launch service {service['id']} has no launch runbook")

    dashboards = records(policy.get("dashboards"), "dashboard contracts")
    exact(
        {item["id"] for item in dashboards},
        {"portfolio_and_edge", "render_and_worker", "data_and_media", "security_and_release", "capacity_and_cost"},
        "dashboard contracts",
    )
    dashboard_boundaries = {boundary for item in dashboards for boundary in item.get("boundaries", [])}
    required_boundaries = {
        "portfolio",
        "dns_tls_edge",
        "edge_route_cache",
        "render_web",
        "worker_api",
        "d1",
        "r2",
        "managed_media",
        "native_media",
        "public_playback",
        "auth_privacy",
        "cache_privacy",
        "release_contract",
    }
    if not required_boundaries <= dashboard_boundaries:
        raise CheckError("dashboard contracts do not distinguish every launch boundary")
    for dashboard in dashboards:
        if not dashboard.get("panels"):
            raise CheckError(f"dashboard {dashboard['id']} has no actionable panels")

    alerts = records(policy.get("alerts"), "launch alerts")
    exact({item["boundary"] for item in alerts}, required_boundaries - {"cache_privacy"}, "alert boundaries")
    for alert in alerts:
        if not isinstance(alert.get("target_ms"), int) or alert["target_ms"] > 60000:
            raise CheckError(f"alert {alert['id']} exceeds the launch detection target")
        if alert.get("owner") not in expected_roles or not str(alert.get("runbook", "")).startswith("docs/operations/subdomain-launch.md#"):
            raise CheckError(f"alert {alert['id']} lacks owner or launch runbook")

    synthetics = records(policy.get("synthetics"), "synthetic monitors")
    exact(
        {item["id"] for item in synthetics},
        {"portfolio_link", "landing", "api", "auth_boundary", "upload_process", "public_playback", "cache_privacy", "portfolio_degradation"},
        "synthetic monitors",
    )
    required_upload = {"upload_intent", "direct_r2_put", "finalize", "process", "deterministic_derivative", "retention_cleanup"}
    upload = next(item for item in synthetics if item["id"] == "upload_process")
    if set(upload.get("steps", [])) != required_upload:
        raise CheckError("synthetic upload omits direct transit, deterministic output, or cleanup")

    release = policy.get("release_join", {})
    exact(
        set(release.get("required_fields", [])),
        {"source_git_sha", "contract_major", "worker_release", "render_deploy", "migration_level", "portfolio_consumer"},
        "release join fields",
    )
    if release.get("current_and_n_minus_1_required") is not True or release.get("production_endpoint_or_header_evidence") != "not_collected":
        raise CheckError("release join omits N/N-1 or overclaims live metadata")

    capacity = policy.get("capacity", {})
    if capacity.get("minimum_headroom_basis_points") != 3000 or capacity.get("protected_cost_budget_microunits") is not None:
        raise CheckError("capacity or protected cost policy drifted")
    exact(
        set(capacity.get("dimensions", [])),
        {"startup", "ssr_requests", "concurrent_requests", "upload_intents", "queued_jobs", "playback_bytes"},
        "capacity dimensions",
    )
    budgets = records(capacity.get("provider_budget_classes"), "provider budget classes")
    exact(
        {item["id"] for item in budgets},
        {"render", "cloudflare_worker", "d1", "r2", "managed_media", "native_media"},
        "provider budget classes",
    )
    for budget in budgets:
        if (
            budget.get("owner") not in expected_roles
            or not budget.get("units")
            or budget.get("numeric_cap") is not None
            or budget.get("quota_status") != "not_collected"
        ):
            raise CheckError(f"provider budget {budget['id']} is unowned or fabricates approval")
    portfolio = policy.get("portfolio_independence", {})
    exact(
        set(portfolio.get("required_faults", [])),
        {"frame_dns", "render", "worker", "d1", "r2", "managed_media", "native_media"},
        "portfolio outage faults",
    )
    if portfolio.get("minimum_availability") != 0.999 or portfolio.get("maximum_latency_regression_basis_points") != 500:
        raise CheckError("portfolio baseline thresholds drifted")

    sequence = policy.get("launch_sequence")
    if not isinstance(sequence, list) or len(sequence) != 9 or len(sequence) != len(set(sequence)):
        raise CheckError("staged launch sequence must contain nine exact unique gates")
    rollbacks = records(policy.get("rollback_layers"), "rollback layers")
    expected_rollbacks = {
        "portfolio_status_off",
        "portfolio_link_remove",
        "optional_browser_off",
        "edge_rules_off",
        "worker_route_remove",
        "render_rollback",
        "proxy_dns_restore",
        "d1_forward_fix",
        "media_fallback",
        "credential_rotation",
    }
    exact({item["id"] for item in rollbacks}, expected_rollbacks, "rollback layers")
    for rollback in rollbacks:
        if rollback.get("owner") not in expected_roles or not rollback.get("verification") or not rollback.get("data_effect"):
            raise CheckError(f"rollback {rollback['id']} lacks owner, verification, or data effect")
        if rollback.get("maximum_ms", 0) <= 0 or rollback["maximum_ms"] > 900000:
            raise CheckError(f"rollback {rollback['id']} has an invalid time target")

    exact(
        set(policy.get("post_launch_decisions", [])),
        {"default_render_hostname", "portfolio_status", "auth_handoff", "browser_cors", "public_embed", "legacy_paths"},
        "post-launch decisions",
    )


def validate_game(game: dict[str, Any], policy: dict[str, Any]) -> None:
    if game.get("evidence_class") != "local_semantic_simulation" or game.get("uses_generated_media_only") is not True:
        raise CheckError("launch game mislabels its local/generated evidence")
    if game.get("provider_or_production_access") is not False:
        raise CheckError("local launch game claims provider access")
    migrations = sorted((ROOT / "apps/control-plane/migrations").glob("*.sql"))
    if not migrations:
        raise CheckError("launch release join has no D1 migration inventory")
    latest_migration = migrations[-1].name
    if any(item.get("migration_level") != latest_migration for item in game.get("release_cases", [])):
        raise CheckError("launch release cases do not name the latest D1 migration")
    exact(
        {item.get("id") for item in game.get("journeys", [])},
        {item["id"] for item in policy["synthetics"]},
        "game journeys",
    )
    exact(
        {item.get("boundary") for item in game.get("failure_injections", [])},
        {item["boundary"] for item in policy["alerts"]},
        "game failure boundaries",
    )
    exact(
        {item.get("id") for item in game.get("rollback_games", [])},
        {item["id"] for item in policy["rollback_layers"]},
        "game rollback layers",
    )


def validate_protected(protected: dict[str, Any], policy: dict[str, Any]) -> None:
    if protected.get("production_authority_changed") is not False or protected.get("local_evidence_may_replace_protected_records") is not False:
        raise CheckError("protected launch ledger does not preserve the authority boundary")
    evidence = records(protected.get("records"), "protected launch records")
    expected_ids = {
        "p7-dependency-and-defect-signoff",
        "dashboard-alert-and-on-call-evidence",
        "provider-synthetic-history",
        "release-version-join-and-n-minus-1",
        "capacity-cost-and-quota-approval",
        "cache-privacy-and-telemetry-audit",
        "dns-tls-route-and-zone-nonregression",
        "portfolio-outage-baseline",
        "timed-full-rollback-game-day",
        "signed-launch-observation-and-post-launch-review",
    }
    exact({item["id"] for item in evidence}, expected_ids, "protected launch records")
    owner_roles = set(policy["owners"].values())
    allowed_commands = (
        "python3 -I scripts/ci/launch-go-no-go.py ",
        "python3 -I scripts/ci/release-join-conformance.py ",
        "scripts/ci/smoke-canonical-domain.sh ",
        "python3 -I scripts/ci/same-origin-live-conformance.py ",
    )
    for item in evidence:
        command = item.get("collection_command")
        if item.get("status") != "not_collected" or item.get("owner_role") not in owner_roles:
            raise CheckError(f"protected record {item['id']} is overclaimed or unowned")
        if not isinstance(command, str) or not command.startswith(allowed_commands) or "/protected/frame-launch/" not in command:
            raise CheckError(f"protected record {item['id']} lacks an exact bounded collection command")
        if not item.get("requires") or not item.get("blocks"):
            raise CheckError(f"protected record {item['id']} lacks requirements or blockers")


def validate_files() -> None:
    required_docs = {
        "fixtures/launch-observability/v1/README.md",
        "docs/operations/subdomain-launch.md",
        "docs/evidence/subdomain-launch-local.md",
        "docs/operations/release-and-cutover.md",
        "docs/operations/service-reliability-and-incidents.md",
        "docs/operations/capacity-cost-residency.md",
        "docs/operations/progressive-cutover-decommission.md",
        "docs/operations/render-web-service.md",
        "docs/operations/same-origin-routing.md",
        "docs/operations/cloudflare-cache-security.md",
        "docs/operations/browser-security-rollout.md",
        "docs/operations/cross-repository-preview.md",
    }
    for relative in required_docs:
        path = ROOT / relative
        if not path.is_file() or path.stat().st_size < 600:
            raise CheckError(f"launch dependency runbook/evidence is missing or empty: {relative}")
    for relative in (
        "scripts/ci/launch-game-day.py",
        "scripts/ci/launch-go-no-go.py",
        "scripts/ci/release-join-conformance.py",
    ):
        path = ROOT / relative
        if not path.is_file() or not path.stat().st_mode & stat.S_IXUSR:
            raise CheckError(f"launch executable is missing or not executable: {relative}")

    workflow_markers = {
        ".github/workflows/quality-gates.yml": ["check-launch-observability.py"],
        ".github/workflows/production-gate.yml": ["check-launch-observability.py"],
        ".github/workflows/operational-hardening.yml": [
            "check-launch-observability.py",
            "launch-game-day.py",
            "launch-go-no-go.py",
            "release-join-conformance.py",
            "launch-game-local.json",
            "launch-go-no-go-self-test.json",
        ],
    }
    for relative, markers in workflow_markers.items():
        text = (ROOT / relative).read_text(encoding="utf-8")
        for marker in markers:
            if marker not in text:
                raise CheckError(f"{relative} omits required launch gate {marker}")
    package = (ROOT / "scripts/ci/package-release.sh").read_text(encoding="utf-8")
    for marker in ("git_sha", "contract_major", "migration_level", "portfolio_consumer_sha"):
        if marker not in package:
            raise CheckError(f"release manifest omits launch join base field {marker}")
    web = (ROOT / "apps/web/src/lib.rs").read_text(encoding="utf-8")
    config = (ROOT / "apps/web/src/config.rs").read_text(encoding="utf-8")
    blueprint = (ROOT / "render.yaml").read_text(encoding="utf-8")
    for marker in (
        '"/health/release"',
        "source_git_sha",
        "worker_release",
        "render_deploy",
        "migration_level",
        "portfolio_consumer",
        "diagnostic_authorized",
        "IncompleteReleaseHealth",
    ):
        if marker not in web:
            raise CheckError(f"web release diagnostic omits {marker}")
    for marker in (
        "FRAME_WORKER_RELEASE",
        "FRAME_RENDER_DEPLOY",
        "FRAME_MIGRATION_LEVEL",
        "FRAME_PORTFOLIO_CONSUMER",
        "IncompleteReleaseJoin",
        "InvalidReleaseJoin",
    ):
        if marker not in config or marker not in blueprint and marker.startswith("FRAME_"):
            raise CheckError(f"release-join configuration omits {marker}")
    latest_migration = sorted((ROOT / "apps/control-plane/migrations").glob("*.sql"))[-1].name
    if latest_migration not in config or latest_migration not in web:
        raise CheckError("web release diagnostic tests do not bind the latest D1 migration")


def run_local_gates() -> None:
    commands = [
        [sys.executable, "-I", "scripts/ci/launch-game-day.py", "--self-test"],
        [sys.executable, "-I", "scripts/ci/launch-go-no-go.py", "--self-test"],
        [sys.executable, "-I", "scripts/ci/release-join-conformance.py", "--self-test"],
    ]
    for command in commands:
        result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode != 0:
            detail = result.stderr.strip() or result.stdout.strip()
            raise CheckError(f"local launch executable failed: {' '.join(command[2:])}: {detail}")


def main() -> int:
    try:
        policy = load("launch-policy.json")
        game = load("local-game.json")
        protected = load("protected-evidence.json")
        validate_policy(policy)
        validate_game(game, policy)
        validate_protected(protected, policy)
        validate_files()
        run_local_gates()
        print(
            "launch observability local gate passed: owners/SLOs, correlated boundary alerts, "
            "full synthetics, release drift, capacity/privacy, layered rollback, and exact protected blockers"
        )
        return 0
    except (CheckError, KeyError, OSError, TypeError, ValueError, json.JSONDecodeError) as error:
        print(f"launch observability check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
