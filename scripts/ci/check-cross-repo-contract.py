#!/usr/bin/env python3
"""Validate Frame's cross-repository contract, preview, and evidence policy."""

from __future__ import annotations

import hashlib
import json
import os
import re
import sys
from pathlib import Path
from typing import Any


ROOT = Path(
    os.environ.get("FRAME_CROSS_REPO_ROOT", Path(__file__).resolve().parents[2])
).resolve()
POLICY_PATH = ROOT / "fixtures/cross-repo-preview/v1/ci-policy.json"
CASES_PATH = ROOT / "fixtures/cross-repo-preview/v1/compatibility-cases.json"
CONTRACT_PATH = ROOT / "fixtures/cross-repo-preview/v1/contract.json"
WORKFLOW_PATH = ROOT / ".github/workflows/cross-repository-contract.yml"
PORTFOLIO_SHA = "1de52bc8f25793dea3697e67765d53785c05cdfa"
ACTION_CHECKOUT_SHA = "34e114876b0b11c390a56381ad16ebd13914f8d5"


def require(condition: bool, message: str, errors: list[str]) -> None:
    if not condition:
        errors.append(message)


def read_object(path: Path, errors: list[str]) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        errors.append(f"{path.relative_to(ROOT)}: invalid UTF-8 JSON ({type(error).__name__})")
        return {}
    if not isinstance(value, dict):
        errors.append(f"{path.relative_to(ROOT)}: must contain an object")
        return {}
    return value


def validate_contract_inventory(policy: dict[str, Any], errors: list[str]) -> None:
    canonical = policy.get("canonical_contract", {})
    require(
        isinstance(canonical, dict)
        and set(canonical)
        == {
            "owner_repository",
            "directory",
            "version",
            "version_policy",
            "consumer_copy_requires",
            "inventory",
        },
        "canonical contract policy fields drifted",
        errors,
    )
    if not isinstance(canonical, dict):
        return
    require(
        canonical.get("owner_repository") == "eng-manager-xyz/frame"
        and canonical.get("directory") == "fixtures/frame-api/v1"
        and canonical.get("version") == 1
        and canonical.get("version_policy")
        == "append_new_major_never_rewrite_released_major",
        "canonical fixture ownership/version policy drifted",
        errors,
    )
    require(
        canonical.get("consumer_copy_requires")
        == [
            "source_repository",
            "source_commit_sha",
            "fixture_set_sha256",
            "drift_check",
        ],
        "consumer copy provenance requirements drifted",
        errors,
    )
    inventory = canonical.get("inventory", {})
    fixture_root = ROOT / "fixtures/frame-api/v1"
    actual_names = {path.name for path in fixture_root.glob("*.json")}
    require(
        isinstance(inventory, dict) and set(inventory) == actual_names,
        "canonical fixture digest inventory drifted",
        errors,
    )
    if isinstance(inventory, dict):
        for name, expected in inventory.items():
            path = fixture_root / name
            actual = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else ""
            require(
                isinstance(expected, str)
                and re.fullmatch(r"[0-9a-f]{64}", expected) is not None
                and actual == expected,
                f"canonical fixture digest drifted: {name}",
                errors,
            )


def validate_compatibility(policy: dict[str, Any], errors: list[str]) -> None:
    matrix = policy.get("producer_consumer_matrix")
    expected_ids = [
        "frame_candidate_current_frame",
        "frame_candidate_last_released_portfolio",
        "released_frame_portfolio_candidate",
        "frame_main_portfolio_default",
        "frame_current_and_n_minus_one",
    ]
    require(isinstance(matrix, list), "producer/consumer matrix is missing", errors)
    if isinstance(matrix, list):
        require(
            [row.get("id") for row in matrix if isinstance(row, dict)] == expected_ids,
            "producer/consumer matrix inventory or order drifted",
            errors,
        )
        rows = [row for row in matrix if isinstance(row, dict)]
        require(
            all(
                set(row)
                == {
                    "id",
                    "producer",
                    "consumer",
                    "trigger",
                    "toolchain_owner",
                    "cache_namespace",
                    "evidence",
                }
                for row in rows
            ),
            "producer/consumer matrix fields drifted",
            errors,
        )
        require(
            len({row.get("cache_namespace") for row in rows}) == len(expected_ids),
            "producer/consumer caches are not isolated",
            errors,
        )
        require(
            rows
            and rows[0].get("evidence") == "required_untrusted_local"
            and all(str(row.get("evidence", "")).startswith("protected_") for row in rows[1:]),
            "local and protected compatibility evidence classes are conflated",
            errors,
        )

    cases = read_object(CASES_PATH, errors)
    require(
        set(cases) == {"schema_version", "base_health_fixture", "base_share_fixture", "cases"}
        and cases.get("schema_version") == 1,
        "compatibility case envelope drifted",
        errors,
    )
    expected_cases = [
        "additive_unknown_health_field",
        "breaking_required_field_removal",
        "breaking_major_version_change",
        "breaking_release_type_change",
        "breaking_status_semantic_change",
        "breaking_public_media_path_change",
    ]
    rows = cases.get("cases")
    require(isinstance(rows, list), "compatibility cases are missing", errors)
    if isinstance(rows, list):
        require(
            [row.get("id") for row in rows if isinstance(row, dict)] == expected_cases,
            "seeded compatibility cases drifted",
            errors,
        )
        for index, row in enumerate(rows):
            if not isinstance(row, dict):
                errors.append(f"compatibility case {index} is not an object")
                continue
            compatible = row.get("classification") == "compatible"
            require(
                row.get("expected_current_consumer")
                == ("accept" if compatible else "reject")
                and row.get("expected_last_released_consumer")
                == ("accept" if compatible else "reject"),
                f"compatibility expectation is unsafe: {row.get('id')}",
                errors,
            )
        require(
            sum(
                isinstance(row, dict) and row.get("classification") == "compatible"
                for row in rows
            )
            == 1
            and sum(
                isinstance(row, dict) and row.get("classification") == "breaking"
                for row in rows
            )
            == 5,
            "compatibility matrix must retain one additive pass and five breaking rejections",
            errors,
        )

    client = (ROOT / "crates/frame-client/src/dto.rs").read_text(encoding="utf-8")
    require(
        "compatibility-cases.json" in client
        and "seeded_producer_changes_match_the_current_consumer_contract" in client
        and all(case_id in client for case_id in expected_cases),
        "frame-client does not execute every seeded compatibility case",
        errors,
    )

    portfolio = policy.get("portfolio_integration_stage")
    require(
        portfolio
        == {
            "manifest": "fixtures/engmanager-portfolio/v1/static-integration.json",
            "base_commit": PORTFOLIO_SHA,
            "stage": "static_link_only",
            "external_repository_mutated_by_frame": False,
            "frame_client_consumer": False,
            "last_released_consumer_evidence": "not_collected",
        },
        "portfolio static patch was promoted into uncollected consumer evidence",
        errors,
    )
    portfolio_manifest = read_object(
        ROOT / "fixtures/engmanager-portfolio/v1/static-integration.json", errors
    )
    require(
        portfolio_manifest.get("base_commit") == PORTFOLIO_SHA
        and portfolio_manifest.get("integration_mode") == "static-link-only"
        and portfolio_manifest.get("live_frame_data") is False
        and portfolio_manifest.get("frame_client_dependency") is False,
        "Issue 37 static integration manifest conflicts with the consumer boundary",
        errors,
    )


def validate_local_and_protected_policy(policy: dict[str, Any], errors: list[str]) -> None:
    local = policy.get("local_gate", {})
    require(
        local
        == {
            "command": "scripts/frame preview-e2e",
            "attempts": 1,
            "orchestration_timeout_seconds": 20,
            "provider_credentials_allowed": False,
            "production_configuration_allowed": False,
            "uploads_artifacts": False,
            "synthetic_media_max_bytes": 1_048_576,
            "max_response_bytes": 65_536,
            "distinct_origins": 2,
        },
        "credential-free local gate limits drifted",
        errors,
    )
    artifacts = policy.get("artifact_policy", {})
    forbidden = artifacts.get("forbidden_fields", []) if isinstance(artifacts, dict) else []
    require(
        isinstance(artifacts, dict)
        and artifacts.get("successful_summary_retention_days") == 3
        and artifacts.get("first_failure_retention_days") == 14
        and all(
            field in forbidden
            for field in (
                "authorization",
                "cookie",
                "cookie_value",
                "object_key",
                "private_response_body",
                "provider_identifier",
                "signed_url",
                "tenant_id",
                "token",
            )
        ),
        "artifact retention/redaction policy drifted",
        errors,
    )
    retry = policy.get("retry_and_flake_policy", {})
    require(
        isinstance(retry, dict)
        and retry.get("local_attempts") == 1
        and retry.get("protected_max_attempts") == 2
        and retry.get("preserve_first_failure_before_retry") is True
        and retry.get("retry_may_replace_required_result") is False
        and retry.get("quarantine_may_satisfy_required_coverage") is False
        and retry.get("quarantine_required_fields")
        == [
            "assertion",
            "owner",
            "deadline",
            "seed",
            "browser_or_provider_version",
            "release_blocking",
        ],
        "retry/flake quarantine policy drifted",
        errors,
    )
    preview = policy.get("trusted_preview_profile", {})
    require(
        isinstance(preview, dict)
        and preview.get("trigger") == "manual_trusted_only"
        and preview.get("environment") == "cross-repository-preview"
        and preview.get("maximum_lifetime_hours") == 72
        and preview.get("noindex_required") is True
        and preview.get("production_secrets_allowed") is False
        and preview.get("production_data_allowed") is False
        and preview.get("production_dns_mutation_allowed") is False
        and len(preview.get("cleanup_proofs", [])) == 4,
        "trusted preview isolation/cleanup policy drifted",
        errors,
    )
    canary = policy.get("protected_provider_canary", {})
    require(
        isinstance(canary, dict)
        and canary.get("trigger") == "scheduled_or_manual_protected"
        and canary.get("environment") == "cross-repository-canary"
        and canary.get("concurrency") == 1
        and canary.get("maximum_requests") == 100
        and canary.get("maximum_media_bytes") == 1_048_576
        and canary.get("maximum_runtime_minutes") == 20
        and canary.get("monthly_budget_usd") == 25
        and canary.get("generated_data_only") is True
        and canary.get("cleanup_required") is True
        and canary.get("kill_switch_required") is True
        and len(canary.get("journeys", [])) == 7,
        "protected provider canary cost/safety policy drifted",
        errors,
    )
    evidence = policy.get("protected_evidence")
    require(
        isinstance(evidence, list)
        and [row.get("id") for row in evidence if isinstance(row, dict)]
        == [
            "portfolio_consumer_build",
            "trusted_preview_cleanup",
            "real_browser_accessibility_security",
            "provider_canary",
        ]
        and all(
            isinstance(row, dict)
            and set(row) == {"id", "status", "requires"}
            and row.get("status") == "not_collected"
            for row in evidence
        ),
        "protected evidence must remain explicit and uncollected locally",
        errors,
    )


def validate_harness(errors: list[str]) -> None:
    contract = read_object(CONTRACT_PATH, errors)
    harness = (ROOT / "scripts/ci/cross-repo-preview-e2e.py").read_text(encoding="utf-8")
    frame_cli = (ROOT / "scripts/frame").read_text(encoding="utf-8")
    controls = [
        "shared-cookie-domain",
        "private-cache-hit",
        "range-off-by-one",
        "unavailable-title-leak",
        "handler-path-upstream-fetch",
        "audit-sensitive-field",
    ]
    require(contract.get("negative_controls") == controls, "harness control inventory drifted", errors)
    require(
        all(control in harness for control in controls)
        and "for _attempt in range(3)" in harness
        and 'path == "/login"' in harness
        and 'path == "/dashboard"' in harness
        and 'path == "/logout"' in harness
        and "portfolio-retry-recovery" in harness
        and "listen_port=old_port" in harness
        and "safe_audit_fields" in harness
        and "privacy_started" in harness,
        "two-origin fault/cache/auth/reconnect/redaction journey is incomplete",
        errors,
    )
    require(
        "preview-e2e)" in frame_cli
        and "cross-repo-preview-e2e.py --self-test --timeout 20" in frame_cli,
        "scripts/frame preview-e2e no longer runs the one-attempt control suite",
        errors,
    )
    require(
        "refuse_provider_state()" in harness
        and '"FRAME_DEPLOYMENT" == "production"' not in harness
        and "start_new_session=True" in harness
        and "process groups stopped" in harness,
        "local harness credential or cleanup boundary drifted",
        errors,
    )


def validate_workflow_and_docs(errors: list[str]) -> None:
    workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
    require(
        "pull_request:" in workflow
        and "push:" in workflow
        and "schedule:" in workflow
        and "workflow_dispatch:" in workflow
        and "permissions:\n  contents: read" in workflow
        and "${{ secrets." not in workflow
        and "continue-on-error: true" not in workflow,
        "Frame cross-repository workflow is privileged or incomplete",
        errors,
    )
    require(
        "shared-key: frame-contract" in workflow
        and "shared-key: portfolio-consumer" in workflow
        and "cargo test --locked -p frame-client --all-features" in workflow
        and "--target wasm32-unknown-unknown" in workflow
        and "scripts/frame preview-e2e" in workflow
        and "test-cross-repo-contract.py" in workflow,
        "Frame producer/current-consumer workflow gate is incomplete",
        errors,
    )
    require(
        workflow.count('"fixtures/engmanager-portfolio/v1/**"') == 2
        and workflow.count('"scripts/ci/check-cross-repo-contract.py"') == 2
        and workflow.count('"scripts/ci/test-cross-repo-contract.py"') == 2
        and workflow.count('"scripts/ci/verify-portfolio-consumer.py"') == 2
        and workflow.count('"scripts/frame"') == 2,
        "cross-repository workflow path filters omit policy or portfolio inputs",
        errors,
    )
    require(
        "environment: cross-repository-consumer" in workflow
        and "github.event_name == 'schedule'" in workflow
        and "inputs.run_external_consumer" in workflow
        and "repository: matthewharwood/engmanager.xyz" in workflow
        and f"ref: {PORTFOLIO_SHA}" in workflow
        and "persist-credentials: false" in workflow
        and "nightly-2026-05-08" in workflow
        and "test -f portfolio/Cargo.lock" in workflow
        and "verify-portfolio-consumer.py" in workflow
        and "cargo test --locked -p website --test frame_contract" in workflow,
        "protected last-released portfolio consumer job is incomplete",
        errors,
    )
    require(
        workflow.count(f"actions/checkout@{ACTION_CHECKOUT_SHA}") == 3,
        "cross-repository workflow checkout actions are not immutably pinned",
        errors,
    )
    verifier = (ROOT / "scripts/ci/verify-portfolio-consumer.py").read_text(encoding="utf-8")
    require(
        "FRAME_CONTRACT_FIXTURE_ROOT" in verifier
        and "fixture_set_sha256" in verifier
        and "source_commit_sha" in verifier
        and "subprocess.run" in verifier
        and "shell=True" not in verifier,
        "portfolio consumer verification is mutable or incomplete",
        errors,
    )
    quality = (ROOT / ".github/workflows/quality-gates.yml").read_text(encoding="utf-8")
    production = (ROOT / ".github/workflows/production-gate.yml").read_text(encoding="utf-8")
    require(
        "check-cross-repo-contract.py" in quality
        and "test-cross-repo-contract.py" in quality,
        "quality gate does not enforce the cross-repository policy and mutations",
        errors,
    )
    require(
        "check-cross-repo-contract.py" in production,
        "production preflight does not enforce cross-repository policy",
        errors,
    )
    docs = "\n".join(
        (ROOT / path).read_text(encoding="utf-8")
        for path in (
            "docs/architecture/cross-repository-contract-ci.md",
            "docs/operations/cross-repository-preview.md",
            "docs/evidence/cross-repo-preview-local.md",
        )
    )
    for phrase in (
        "first-failure",
        "no production secret",
        "concurrency one",
        "last released portfolio consumer",
        "not a browser or provider emulator",
        "protected evidence",
    ):
        require(phrase.lower() in docs.lower(), f"cross-repository docs omit {phrase!r}", errors)


def main() -> int:
    errors: list[str] = []
    policy = read_object(POLICY_PATH, errors)
    require(
        set(policy)
        == {
            "schema_version",
            "evidence_class",
            "canonical_contract",
            "producer_consumer_matrix",
            "portfolio_integration_stage",
            "local_gate",
            "artifact_policy",
            "retry_and_flake_policy",
            "trusted_preview_profile",
            "protected_provider_canary",
            "protected_evidence",
        }
        and policy.get("schema_version") == 1
        and policy.get("evidence_class") == "executable_policy_definition",
        "cross-repository policy envelope drifted",
        errors,
    )
    validate_contract_inventory(policy, errors)
    validate_compatibility(policy, errors)
    validate_local_and_protected_policy(policy, errors)
    validate_harness(errors)
    validate_workflow_and_docs(errors)
    if errors:
        print("cross-repository contract policy failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(
        "validated 5 producer/consumer lanes, 6 seeded compatibility cases, "
        "2-origin controls, and protected preview/canary policy"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
