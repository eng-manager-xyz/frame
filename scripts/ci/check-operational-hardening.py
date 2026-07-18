#!/usr/bin/env python3
"""Validate and execute the complete local Issue-34 operational contract."""

from __future__ import annotations

import json
import pathlib
import stat
import subprocess
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures/operational-hardening/v1"


class CheckError(RuntimeError):
    pass


def load(name: str) -> dict[str, Any]:
    path = FIXTURE / name
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise CheckError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict) or value.get("schema_version") != 1:
        raise CheckError(f"{path} must be a schema-version 1 object")
    return value


def exact(actual: set[str], expected: set[str], label: str) -> None:
    if actual != expected:
        raise CheckError(
            f"{label} mismatch; missing={sorted(expected - actual)}, extra={sorted(actual - expected)}"
        )


def validate_catalog(catalog: dict[str, Any]) -> None:
    services = catalog.get("services")
    if not isinstance(services, list):
        raise CheckError("service catalog services must be a list")
    expected_services = {
        "control_plane_worker",
        "d1_metadata",
        "r2_objects",
        "media_execution",
        "native_media_worker",
        "managed_media_provider",
        "web_service",
        "desktop_client",
        "frame_client",
    }
    exact({item.get("id") for item in services}, expected_services, "services")
    required_boundaries = {
        "worker",
        "d1",
        "queue_job",
        "object",
        "media_worker",
        "desktop_update",
        "client",
    }
    if not required_boundaries <= {item.get("failure_boundary") for item in services}:
        raise CheckError("service catalog omits a required acceptance failure boundary")
    roles = set(catalog["ownership_model"]["required_roles"])
    if catalog["ownership_model"].get("repository_contains_personal_contacts") is not False:
        raise CheckError("service ownership must not embed personal contacts")
    for service in services:
        if service.get("owner_role") not in roles:
            raise CheckError(f"service {service.get('id')} has no cataloged owner role")
        source = ROOT / service["source"]
        if not source.exists():
            raise CheckError(f"service source does not exist: {source}")
        if not service.get("slo_ids") or not service.get("safe_operations"):
            raise CheckError(f"service {service['id']} lacks SLOs or safe operations")
    slos = catalog.get("slos")
    if not isinstance(slos, list):
        raise CheckError("SLO inventory must be a list")
    expected_slos = {
        "landing_availability",
        "api_availability",
        "landing_latency",
        "api_latency",
        "upload_finalize_success",
        "playback_start",
        "time_to_share",
        "reconciliation",
        "durable_state_rpo",
        "service_rto",
        "desktop_update_success",
        "client_failure_rate",
    }
    exact({item.get("id") for item in slos}, expected_slos, "SLOs")
    by_id = {item["id"]: item for item in slos}
    expected_values = {
        "landing_availability": ("objective", 0.999),
        "api_availability": ("objective", 0.999),
        "landing_latency": ("maximum", 750),
        "api_latency": ("maximum", 500),
        "upload_finalize_success": ("objective", 0.999),
        "playback_start": ("maximum", 2000),
        "time_to_share": ("maximum", 30000),
        "reconciliation": ("maximum", 0),
        "durable_state_rpo": ("maximum", 300000),
        "service_rto": ("maximum", 3600000),
    }
    for slo_id, (field, value) in expected_values.items():
        if by_id[slo_id].get(field) != value or not by_id[slo_id].get("alert"):
            raise CheckError(f"SLO {slo_id} drifts from the charter or lacks an alert")
    dashboard = catalog["dashboard_contract"]
    if dashboard.get("screenshot_status") != "protected_not_collected":
        raise CheckError("dashboard screenshots may not be fabricated locally")
    if "tenant" not in dashboard.get("forbidden_dimensions", []):
        raise CheckError("dashboard contract does not forbid tenant cardinality")


def validate_policy(policy: dict[str, Any]) -> None:
    release = policy["release"]
    expected_deployables = {
        "control_plane_worker",
        "render_web_service",
        "linux_native_worker",
        "desktop_macos",
        "desktop_windows",
        "rust_client_package",
    }
    exact({item.get("id") for item in release["deployables"]}, expected_deployables, "deployables")
    for deployable in release["deployables"]:
        if deployable.get("signature_required") is not True or len(deployable.get("promotion", [])) < 3:
            raise CheckError(f"deployable {deployable['id']} lacks signing/promotion policy")
        if not deployable.get("rollback") or not deployable.get("sbom"):
            raise CheckError(f"deployable {deployable['id']} lacks rollback/SBOM policy")
    rules = release["promotion_rules"]
    for field in (
        "same_subject_digest_across_stages",
        "separation_of_duties",
        "production_environment_review",
        "rollback_pointer_required",
        "first_failure_retained",
    ):
        if rules.get(field) is not True:
            raise CheckError(f"release promotion rule is not fail closed: {field}")
    if any(value != "not_collected" for value in release["protected_status"].values()):
        raise CheckError("release policy fabricates protected release evidence")

    schema = policy["telemetry"]["event_schema"]
    exact(
        set(schema["required"]),
        {
            "schema_version",
            "timestamp_ms",
            "service",
            "release_id",
            "environment",
            "operation",
            "result_class",
            "duration_ms",
            "correlation_id",
        },
        "diagnostic required fields",
    )
    required_forbidden = {
        "media_bytes",
        "tokens",
        "signed_urls",
        "raw_email",
        "captions",
        "object_keys",
        "tenant_or_user_identifiers",
    }
    if not required_forbidden <= set(policy["telemetry"]["forbidden_data"]):
        raise CheckError("telemetry policy omits forbidden personal/media data")

    expected_rotation = {
        "cloudflare_deploy",
        "r2_signing",
        "session_hash",
        "webhook_hmac",
        "desktop_update",
        "backup_recovery",
    }
    exact({item.get("id") for item in policy["security"]["rotation_classes"]}, expected_rotation, "rotation classes")
    required_vulnerability = {
        "cargo_deny_advisories",
        "cargo_deny_bans",
        "cargo_deny_licenses",
        "cargo_deny_sources",
        "secret_scan",
        "cyclonedx_1_6",
        "native_runtime_inventory",
    }
    if not required_vulnerability <= set(policy["security"]["vulnerability_gates"]):
        raise CheckError("security policy omits vulnerability/license/SBOM gates")
    if policy["security"]["penetration_test"].get("status") != "not_collected":
        raise CheckError("penetration evidence may not be fabricated locally")

    recovery = policy["recovery"]
    if recovery.get("rpo_ms") != 300000 or recovery.get("rto_ms") != 3600000:
        raise CheckError("recovery targets drift from the charter")
    exact(
        {item.get("id") for item in recovery["assets"]},
        {"d1_export", "object_manifest", "configuration", "signing_key_catalog", "desktop_projects"},
        "recovery assets",
    )
    if any(value != "not_collected" for value in recovery["protected_status"].values()):
        raise CheckError("recovery policy fabricates protected evidence")
    if policy["capacity_cost_residency"].get("minimum_headroom_basis_points") != 3000:
        raise CheckError("capacity headroom drifts from the charter")
    cost = policy["capacity_cost_residency"]["provider_cost"]
    if cost.get("protected_maximum_microunits") is not None or cost.get("concurrency") != 1:
        raise CheckError("provider cost must require external numeric approval and concurrency one")


def validate_media(policy: dict[str, Any]) -> None:
    catalog = json.loads((ROOT / "fixtures/media-jobs/v1/catalog.json").read_text(encoding="utf-8"))
    hybrid = {
        item["profile"]
        for item in catalog["jobs"]
        if item.get("disposition") == "hybrid_managed_native"
    }
    configured = {item["profile"] for item in policy["managed_media"]["managed_profiles"]}
    exact(configured, hybrid, "managed Media profiles")
    for profile in policy["managed_media"]["managed_profiles"]:
        if profile.get("kill_switch") != "per_profile_revision" or profile.get("fallback") != "native_gstreamer":
            raise CheckError(f"managed profile {profile['profile']} lacks exact kill switch/fallback")
    required_alerts = {
        "quota_exhaustion",
        "provider_outage",
        "managed_output_drift",
        "usage_cost_budget",
        "duplicate_publication",
        "manifest_object_drift",
    }
    if not required_alerts <= set(policy["managed_media"]["alerts"]):
        raise CheckError("managed Media alert inventory is incomplete")
    if policy["managed_media"]["change_watch"].get("maximum_review_interval_days") != 7:
        raise CheckError("managed Media change watch is not weekly")


def validate_protected(protected: dict[str, Any]) -> None:
    records = protected.get("records")
    if not isinstance(records, list) or len(records) != 12:
        raise CheckError("protected evidence ledger must enumerate twelve exact blockers")
    if len({item.get("id") for item in records}) != len(records):
        raise CheckError("protected evidence records have duplicate IDs")
    for record in records:
        if record.get("status") != "not_collected" or not record.get("requires") or not record.get("blocks"):
            raise CheckError(f"protected evidence record overclaims completion: {record.get('id')}")
    if protected.get("local_evidence_may_replace_protected_records") is not False:
        raise CheckError("local evidence may not replace protected records")


def validate_files() -> None:
    required_docs = {
        "docs/operations/release-provenance-and-promotion.md",
        "docs/operations/service-reliability-and-incidents.md",
        "docs/operations/backup-restore-dr.md",
        "docs/operations/capacity-cost-residency.md",
        "docs/operations/media-provider-game-day.md",
        "docs/security/operational-security.md",
        "docs/evidence/operational-hardening-local.md",
    }
    for relative in required_docs:
        path = ROOT / relative
        if not path.is_file() or path.stat().st_size < 1000:
            raise CheckError(f"operational runbook/evidence is missing or empty: {relative}")
    scripts = {
        "scripts/ci/release-provenance.py",
        "scripts/ci/support-bundle.py",
        "scripts/ci/restore-dr-rehearsal.py",
        "scripts/ci/operational-game.py",
    }
    for relative in scripts:
        path = ROOT / relative
        if not path.is_file() or not path.stat().st_mode & stat.S_IXUSR:
            raise CheckError(f"operational gate is missing or not executable: {relative}")
    workflow = (ROOT / ".github/workflows/operational-hardening.yml").read_text(encoding="utf-8")
    for marker in (
        "check-operational-hardening.py",
        "restore-dr-rehearsal.py",
        "operational-game.py",
        "support-bundle.py --self-test",
        "release-provenance.py --self-test",
    ):
        if marker not in workflow:
            raise CheckError(f"operational workflow omits {marker}")
    package = (ROOT / "scripts/ci/package-release.sh").read_text(encoding="utf-8")
    verify = (ROOT / "scripts/ci/verify-release-bundle.sh").read_text(encoding="utf-8")
    for marker in ("release-manifest.json", "frame.cdx.json", "SHA256SUMS"):
        if marker not in package or marker not in verify:
            raise CheckError(f"release bundle does not create and verify {marker}")


def run_local_executables() -> None:
    commands = [
        [sys.executable, "-I", "scripts/ci/release-provenance.py", "--self-test"],
        [sys.executable, "-I", "scripts/ci/support-bundle.py", "--self-test"],
        [sys.executable, "-I", "scripts/ci/restore-dr-rehearsal.py"],
        [sys.executable, "-I", "scripts/ci/operational-game.py"],
    ]
    for command in commands:
        result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode != 0:
            detail = result.stderr.strip() or result.stdout.strip()
            raise CheckError(f"local operational executable failed: {' '.join(command[2:])}: {detail}")


def main() -> int:
    try:
        catalog = load("service-catalog.json")
        policy = load("operational-policy.json")
        protected = load("protected-evidence.json")
        load("game-scenarios.json")
        validate_catalog(catalog)
        validate_policy(policy)
        validate_media(policy)
        validate_protected(protected)
        validate_files()
        run_local_executables()
        print(
            "operational hardening local gate passed: release/provenance, service/SLO/privacy, "
            "security/rotation, restore/DR, capacity/residency, Media game, and exact protected blockers"
        )
        return 0
    except (CheckError, KeyError, OSError, json.JSONDecodeError) as error:
        print(f"operational hardening check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
