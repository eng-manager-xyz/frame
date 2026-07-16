#!/usr/bin/env python3
"""Validate the complete local Issue-35 cutover/decommission contract."""

from __future__ import annotations

import json
import pathlib
import stat
import subprocess
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures/cutover-decommission/v1"


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
        raise CheckError(f"{label} mismatch; missing={sorted(expected - actual)}, extra={sorted(actual - expected)}")


def validate_inventory() -> None:
    expected = {
        "README.md",
        "cohorts.json",
        "cutover-policy.json",
        "dashboard-scenarios.json",
        "decommission-plan.json",
        "protected-evidence.json",
        "reconciliation-contract.json",
        "routing-disposition.json",
    }
    exact({path.name for path in FIXTURE.iterdir() if path.is_file()}, expected, "Issue-35 fixture files")


def validate_policy(policy: dict[str, Any]) -> None:
    if policy.get("decision_is_advisory") is not True or policy.get("decision_never_changes_authority") is not True:
        raise CheckError("go/no-go evaluation must remain advisory and non-mutating")
    stages = policy.get("ramp_stages")
    if not isinstance(stages, list):
        raise CheckError("ramp stages must be a list")
    expected_stages = [
        ("preflight", 0, True),
        ("internal", 100, True),
        ("representative_5", 500, True),
        ("representative_25", 2500, True),
        ("majority_50", 5000, True),
        ("full_reversible", 10000, True),
        ("irreversible_finalize", 10000, False),
    ]
    actual_stages = [(item.get("id"), item.get("frame_traffic_basis_points"), item.get("reversible")) for item in stages]
    if actual_stages != expected_stages:
        raise CheckError("ramp sequence, traffic percentages, or reversibility drifted")
    for index, stage in enumerate(stages):
        expected_next = stages[index + 1]["id"] if index + 1 < len(stages) else None
        if stage.get("next") != expected_next or not stage.get("cohorts") or not stage.get("authority_intent"):
            raise CheckError(f"ramp stage is not linked or scoped: {stage.get('id')}")
        if stage["id"] not in {"preflight", "irreversible_finalize"} and stage.get("minimum_observation_ms", 0) < 86400000:
            raise CheckError(f"canary stage lacks an observation window: {stage['id']}")
    exact(
        {item.get("id") for item in policy.get("authority_dimensions", [])},
        {"metadata", "objects", "routes", "jobs", "storage", "executor", "clients"},
        "authority dimensions",
    )
    for dimension in policy["authority_dimensions"]:
        if not dimension.get("control") or not dimension.get("irreversible_value") or not dimension.get("rollback_before_irreversible"):
            raise CheckError(f"authority dimension lacks control/rollback: {dimension.get('id')}")
    gates = policy.get("gate_definitions")
    if not isinstance(gates, list) or len(gates) < 50:
        raise CheckError("dashboard gate inventory is incomplete")
    codes = [item.get("code") for item in gates]
    if len(codes) != len(set(codes)):
        raise CheckError("dashboard gate codes are not unique")
    required_groups = {"phase_readiness", "slos", "parity", "support", "reconciliation", "backlog", "clients", "capacity", "rollback", "managed_media", "evidence"}
    if not required_groups <= {item.get("group") for item in gates}:
        raise CheckError("dashboard does not combine every required signal family")
    required_codes = {
        "phase_p0_signed", "phase_p5_signed", "critical_blockers_owned_or_closed",
        "public_availability", "api_latency_p95_ms", "privacy_or_corruption_incidents",
        "authorization_parity", "support_budget", "metadata_relationships", "sampled_semantics",
        "object_counts", "object_bytes", "object_checksums", "replay_dead_letters",
        "n_minus_1_client", "capacity_headroom", "rollback_rehearsed", "rollback_time",
        "rollback_preserved_writes", "media_usage_cost", "media_provider_errors",
        "media_fallback_rate", "media_output_drift", "media_duplicate_artifacts", "stage_record",
    }
    if not required_codes <= set(codes):
        raise CheckError("dashboard omits a required release gate")
    gates_by_code = {item["code"]: item for item in gates}
    charter_thresholds = {
        "public_availability": ("gte", 0.999),
        "api_availability": ("gte", 0.999),
        "landing_latency_p95_ms": ("lte", 750),
        "api_latency_p95_ms": ("lte", 500),
        "upload_finalize_success": ("gte", 0.999),
        "playback_start_p95_ms": ("lte", 2000),
        "time_to_share_p95_ms": ("lte", 30000),
        "privacy_or_corruption_incidents": ("lte", 0),
        "metadata_rows": ("lte", 0),
        "metadata_relationships": ("lte", 0),
        "metadata_aggregates": ("lte", 0),
        "metadata_field_hashes": ("lte", 0),
        "sampled_semantics": ("lte", 0),
        "object_counts": ("lte", 0),
        "object_bytes": ("lte", 0),
        "object_checksums": ("lte", 0),
        "capacity_headroom": ("gte", 3000),
        "rollback_time": ("lte", 900000),
    }
    for code, (operator, threshold) in charter_thresholds.items():
        gate = gates_by_code.get(code, {})
        if gate.get("operator") != operator or gate.get("threshold") != threshold:
            raise CheckError(f"dashboard gate drifts from the migration charter: {code}")
    observation = policy.get("observation", {})
    if observation.get("missing_or_stale_data_decision") != "NO_GO" or observation.get("privacy_corruption_duplicate_billing_or_acknowledged_write_loss_budget") != 0:
        raise CheckError("missing evidence or zero-tolerance events do not fail closed")
    irreversible = policy.get("irreversible_gate", {})
    if irreversible.get("automatic_evaluator_may_execute") is not False:
        raise CheckError("automated evaluator may not execute the irreversible gate")
    for field in ("requires_rollback_expiry_approval", "requires_final_reconciliation", "requires_source_retention_approval", "requires_legacy_write_probe_denial", "requires_post_cutover_monitoring"):
        if irreversible.get(field) is not True:
            raise CheckError(f"irreversible gate omits {field}")


def validate_cohorts(cohorts: dict[str, Any], policy: dict[str, Any]) -> None:
    if cohorts.get("repository_contains_tenant_identifiers") is not False:
        raise CheckError("cohort fixture may not contain tenant identifiers")
    rules = cohorts.get("selection_rules", {})
    for field in ("explicit_allowlist_precedes_percentage", "tenant_atomic", "route_family_atomic", "cross_tenant_sampling_forbidden", "assignment_changes_require_new_stage_record"):
        if rules.get(field) is not True:
            raise CheckError(f"cohort selection is not fail closed: {field}")
    expected_ids = {"synthetic", "internal_hosted_web", "internal_desktop_native", "representative_hosted", "representative_byo", "representative_high_risk"}
    exact({item.get("id") for item in cohorts.get("cohorts", [])}, expected_ids, "canary cohorts")
    dimensions = cohorts.get("required_dimensions", {})
    field_map = {
        "storage_mode": "storage_modes",
        "platform": "platforms",
        "browser": "browsers",
        "media_mode": "media_modes",
        "workflow_risk": "workflow_risks",
    }
    for dimension, field in field_map.items():
        expected = set(dimensions.get(dimension, []))
        covered = {value for cohort in cohorts["cohorts"] for value in cohort.get(field, [])}
        if expected != covered:
            raise CheckError(f"cohort coverage is incomplete for {dimension}")
    for cohort in cohorts["cohorts"]:
        if cohort["id"] != "synthetic" and cohort.get("protected_membership_evidence") is not True:
            raise CheckError(f"production-shaped cohort lacks protected membership evidence: {cohort['id']}")
    referenced = {cohort for stage in policy["ramp_stages"] for cohort in stage["cohorts"]}
    exact(referenced, expected_ids, "ramp cohort references")


def validate_routing(routing: dict[str, Any]) -> None:
    catalog = json.loads((ROOT / "fixtures/media-jobs/v1/catalog.json").read_text(encoding="utf-8"))
    source = {item["profile"]: item for item in catalog["jobs"]}
    profiles = routing.get("profiles")
    if not isinstance(profiles, list):
        raise CheckError("routing profiles must be a list")
    exact({item.get("profile") for item in profiles}, set(source), "per-profile routing")
    if routing.get("one_logical_attempt") is not True or routing.get("legacy_remains_available_until_rollback_expiry") is not True:
        raise CheckError("routing plan permits duplicate attempts or early legacy removal")
    flag = routing.get("flag_contract", {})
    if flag.get("compare_and_swap_required") is not True or flag.get("unknown_state_effect") != "deny_new_admission" or flag.get("stale_revision_effect") != "deny_new_admission":
        raise CheckError("profile routing flags do not fail closed")
    in_flight = routing.get("in_flight_policy", {})
    exact(
        {"queued", "claimed_not_started", "provider_or_native_started", "staged_output", "published", "cancel_requested", "indeterminate"},
        {key for key in in_flight if key not in {"id", "rollback_never"}},
        "in-flight dispositions",
    )
    required_never = {"delete_committed_output", "repeat_billable_effect", "guess_provider_result", "publish_partial_output", "route_to_unapproved_executor"}
    exact(set(in_flight.get("rollback_never", [])), required_never, "forbidden rollback effects")
    for profile in profiles:
        source_job = source[profile["profile"]]
        if profile.get("job_id") != source_job["id"] or profile.get("catalog_disposition") != source_job["disposition"]:
            raise CheckError(f"routing plan drifts from media catalog: {profile.get('profile')}")
        expected_fallback = source_job.get("fallback") or "stable_unsupported_or_unavailable"
        if profile.get("frame_primary") != source_job["preferred"] or profile.get("failure_fallback") != expected_fallback:
            raise CheckError(f"routing executor/fallback drifts from media catalog: {profile['profile']}")
        if profile.get("initial_route") != "legacy_adapter" or profile.get("cutover_rollback") != "legacy_adapter":
            raise CheckError(f"profile lacks deterministic cutover rollback: {profile['profile']}")
        if profile.get("kill_switch") != "profile_revision" or profile.get("in_flight_policy") != "fenced_attempt_v1":
            raise CheckError(f"profile lacks a revision kill switch/in-flight policy: {profile['profile']}")
        if profile.get("production_readiness") != "not_collected":
            raise CheckError(f"profile fabricates production readiness: {profile['profile']}")


def validate_reconciliation(contract: dict[str, Any]) -> None:
    metadata = contract.get("mysql_d1", {})
    objects = contract.get("objects", {})
    required_metadata = {"row_count", "primary_key_set", "foreign_key_relationship", "field_hash", "aggregate", "policy_semantic", "sampled_api_behavior", "replay_checkpoint"}
    required_objects = {"object_count", "logical_bytes", "role_count", "full_sha256", "required_media_probe", "missing_source", "missing_target", "duplicate_source", "duplicate_target", "orphan_source", "orphan_target", "ownership_mismatch", "corrupt_or_unplayable", "checkpoint_and_publication_provenance"}
    exact(set(metadata.get("required_dimensions", [])), required_metadata, "MySQL/D1 reconciliation dimensions")
    exact(set(objects.get("required_dimensions", [])), required_objects, "object reconciliation dimensions")
    if metadata.get("maximum_unexplained_differences") != 0 or objects.get("maximum_unexplained_differences") != 0:
        raise CheckError("final reconciliation must require zero unexplained differences")
    if objects.get("independent_complete_inventory_passes_per_side") != 2 or objects.get("provider_etag_is_content_hash") is not False:
        raise CheckError("object final reconciliation lacks independent passes or treats etags as hashes")
    if metadata.get("production_status") != "not_collected" or objects.get("production_status") != "not_collected":
        raise CheckError("final reconciliation evidence is fabricated")
    final = contract.get("final_gate", {})
    for field in ("metadata_and_objects_same_checkpoint_window", "replay_drained_and_fenced", "all_quarantine_dispositions_attached", "sampled_semantic_playback_and_range_probes_required", "report_age_within_signed_stage_budget", "independent_approver_required", "zero_unexplained_differences"):
        if final.get(field) is not True:
            raise CheckError(f"final reconciliation omits {field}")
    if final.get("automatic_cleanup_on_success") is not False:
        raise CheckError("reconciliation success may not automatically clean up sources")


def validate_decommission(plan: dict[str, Any]) -> None:
    expected_categories = {"services", "routes", "queues", "databases", "buckets", "secrets", "dns", "jobs", "clients", "dashboards", "runbooks", "billing"}
    inventory = plan.get("inventory")
    if not isinstance(inventory, list):
        raise CheckError("decommission inventory must be a list")
    exact({item.get("category") for item in inventory}, expected_categories, "decommission categories")
    if plan.get("execution_status") != "planned_not_executed" or plan.get("destructive_actions_automated") is not False:
        raise CheckError("decommission fixture overclaims execution or automates destruction")
    retention = {item.get("id") for item in plan.get("retention_classes", [])}
    exact(retention, {"rollback_source", "migration_evidence", "customer_content"}, "retention classes")
    for item in inventory:
        if item.get("status") != "planned_not_executed":
            raise CheckError(f"decommission item overclaims execution: {item.get('id')}")
        if item.get("retention_class") not in retention or not item.get("owner_role") or not item.get("action") or not item.get("proof") or not item.get("rollback_before_expiry"):
            raise CheckError(f"decommission item lacks owner/action/proof/rollback: {item.get('id')}")
    post = plan.get("post_cutover", {})
    for field in ("monitoring_remains_active", "customer_support_report_required", "cost_delta_required", "retrospective_required", "credential_revocation_receipts_must_exclude_secret_values", "decommission_record_is_immutable"):
        if post.get(field) is not True:
            raise CheckError(f"post-cutover contract omits {field}")


def validate_protected(protected: dict[str, Any]) -> None:
    records = protected.get("records")
    if not isinstance(records, list) or len(records) != 14:
        raise CheckError("protected evidence ledger must enumerate fourteen exact records")
    if len({item.get("id") for item in records}) != len(records):
        raise CheckError("protected evidence IDs are not unique")
    for record in records:
        if record.get("status") != "not_collected" or not record.get("requires") or not record.get("blocks"):
            raise CheckError(f"protected record overclaims completion: {record.get('id')}")
    for field in ("production_authority_changed", "legacy_resource_changed_or_revoked", "local_evidence_may_replace_protected_records"):
        if protected.get(field) is not False:
            raise CheckError(f"protected ledger must keep {field}=false")


def validate_files() -> None:
    required = {
        "docs/operations/progressive-cutover-decommission.md": 5000,
        "docs/evidence/cutover-decommission-local.md": 1200,
    }
    for relative, minimum in required.items():
        path = ROOT / relative
        if not path.is_file() or path.stat().st_size < minimum:
            raise CheckError(f"Issue-35 runbook/evidence is missing or too small: {relative}")
    evaluator = ROOT / "scripts/ci/cutover_go_no_go.py"
    if not evaluator.is_file() or not evaluator.stat().st_mode & stat.S_IXUSR:
        raise CheckError("cutover evaluator is missing or not executable")
    runbook = (ROOT / "docs/operations/progressive-cutover-decommission.md").read_text(encoding="utf-8")
    for marker in (
        "## Command and evidence index",
        "## Staged timeline and checkpoints",
        "## Timed rollback rehearsal",
        "## Final reconciliation",
        "## Irreversible gate",
        "## Legacy retention and decommission",
        "## Communications",
        "## Protected status",
    ):
        if marker not in runbook:
            raise CheckError(f"progressive cutover runbook omits {marker}")
    workflow = (ROOT / ".github/workflows/cutover-decommission.yml").read_text(encoding="utf-8")
    for marker in ("check-cutover-decommission.py", "cutover_go_no_go.py --self-test", "cutover-authority-conformance.py", "cutover_authority_v1"):
        if marker not in workflow:
            raise CheckError(f"cutover workflow omits {marker}")


def run_evaluator() -> None:
    result = subprocess.run(
        [sys.executable, "-I", "scripts/ci/cutover_go_no_go.py", "--self-test"],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        raise CheckError(f"cutover evaluator self-test failed: {detail}")


def main() -> int:
    try:
        validate_inventory()
        policy = load("cutover-policy.json")
        cohorts = load("cohorts.json")
        routing = load("routing-disposition.json")
        reconciliation = load("reconciliation-contract.json")
        decommission = load("decommission-plan.json")
        protected = load("protected-evidence.json")
        load("dashboard-scenarios.json")
        validate_policy(policy)
        validate_cohorts(cohorts, policy)
        validate_routing(routing)
        validate_reconciliation(reconciliation)
        validate_decommission(decommission)
        validate_protected(protected)
        validate_files()
        run_evaluator()
        print(
            "cutover/decommission local gate passed: staged cohorts, 16 profile dispositions, "
            "deterministic dashboard, final reconciliation, retention inventory, and 14 protected blockers"
        )
        return 0
    except (CheckError, KeyError, OSError, json.JSONDecodeError, TypeError, ValueError) as error:
        print(f"cutover/decommission check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
