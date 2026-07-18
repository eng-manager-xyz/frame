#!/usr/bin/env python3
"""Verify exact Worker/D1 authority for protected contract promotion.

The live workflow supplies Cloudflare control-plane responses and a privacy-safe
D1 aggregate. This verifier intentionally has no network or credential access;
its self-test exercises every fail-closed decision locally.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import pathlib
import re
import sys
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
EXPAND_DIR = ROOT / "apps" / "control-plane" / "migrations"
CONTRACT_DIR = ROOT / "apps" / "control-plane" / "contract-migrations"
GIT_SHA = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
PROVIDER_ETAG = re.compile(r"^[A-Za-z0-9._:+/=-]{8,128}$")
PROVIDER_ID = re.compile(
    r"^(?:[0-9a-f]{32}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$"
)
MINIMUM_ACTIVE_AGE_SECONDS = 15 * 60
ROLLBACK_BOOTSTRAP_CONFIRMATION = "adopt-current-unannotated-worker-once"
CONTRACT_COMPATIBILITY_FLOORS = {
    "0032_media_job_inputs_enforce.sql": "0027_media_job_inputs.sql",
    "0033_r2_multipart_claims_enforce.sql": "0028_r2_multipart_part_claims.sql",
}
ZERO_FIELDS = (
    "media_legacy_residual_count",
    "r2_nonterminal_session_count",
    "r2_uncommitted_creation_count",
    "r2_unmaterialized_part_count",
    "r2_active_completion_count",
    "r2_pending_completion_reconciliation_count",
    "r2_pending_abort_reconciliation_count",
)


class AuthorityError(RuntimeError):
    """A protected authority proof failed without exposing provider data."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AuthorityError(message)


def read_json(path: pathlib.Path, label: str) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise AuthorityError(f"{label} is not valid bounded JSON") from error


def api_result(value: Any, label: str) -> Any:
    require(isinstance(value, dict), f"{label} response must be an object")
    require(value.get("success") is True, f"{label} provider request was not successful")
    require(value.get("errors") in (None, []), f"{label} provider response contains errors")
    require("result" in value, f"{label} provider response has no result")
    return value["result"]


def migration_inventory(directory: pathlib.Path) -> list[str]:
    files = sorted(directory.glob("[0-9][0-9][0-9][0-9]_*.sql"))
    require(bool(files), f"{directory.name} migration inventory is empty")
    return [path.name for path in files]


def deployment_inventory(value: Any) -> list[dict[str, Any]]:
    result = api_result(value, "Worker deployments")
    if isinstance(result, dict):
        result = result.get("deployments")
    require(isinstance(result, list) and result, "Worker deployment inventory is empty")
    require(all(isinstance(item, dict) for item in result), "Worker deployment inventory is invalid")
    return result


def deployment_version(deployment: dict[str, Any], expected_version: str) -> None:
    versions = deployment.get("versions")
    require(isinstance(versions, list) and len(versions) == 1, "Worker deployment is not a single-version fence")
    version = versions[0]
    require(isinstance(version, dict), "Worker deployment version entry is invalid")
    require(version.get("version_id") == expected_version, "Worker deployment points at the wrong version")
    require(version.get("percentage") in (100, 100.0), "Worker deployment does not own 100 percent of traffic")


def parse_timestamp(value: Any) -> dt.datetime:
    require(isinstance(value, str) and value.endswith("Z"), "Worker deployment timestamp is invalid")
    try:
        return dt.datetime.fromisoformat(value.removesuffix("Z") + "+00:00")
    except ValueError as error:
        raise AuthorityError("Worker deployment timestamp is invalid") from error


def find_deployment(
    deployments: list[dict[str, Any]], deployment_id: str, version_id: str
) -> dict[str, Any]:
    matches = [item for item in deployments if item.get("id") == deployment_id]
    require(len(matches) == 1, "expected Worker deployment was not observed exactly once")
    deployment_version(matches[0], version_id)
    return matches[0]


def version_result(
    value: Any,
    expected_id: str,
    source_sha: str,
    artifact_sha: str,
    expand_level: str,
    contract_level: str,
    *,
    identity_mode: str = "annotated",
    provider_etag: str | None = None,
    bootstrap_confirmation: str | None = None,
    script_version_value: Any | None = None,
) -> str:
    version = api_result(value, "Worker version")
    require(isinstance(version, dict) and version.get("id") == expected_id, "Worker version identity differs from approval")
    annotations = version.get("annotations")
    expected_message = (
        f"frame-source:{source_sha};frame-worker-sha256:{artifact_sha};"
        f"frame-expand:{expand_level};frame-contract:{contract_level}"
    )
    if identity_mode == "annotated":
        require(isinstance(annotations, dict), "Worker version annotations are unavailable")
        require(annotations.get("workers/tag") == source_sha, "Worker version source tag differs from approval")
        require(annotations.get("workers/message") == expected_message, "Worker version artifact annotation differs from approval")
        return "provider_version_annotations"
    require(
        identity_mode == "protected-unannotated-bootstrap",
        "rollback identity mode is unknown",
    )
    require(
        bootstrap_confirmation == ROLLBACK_BOOTSTRAP_CONFIRMATION,
        "unannotated rollback adoption lacks the exact protected confirmation",
    )
    require(
        isinstance(provider_etag, str)
        and PROVIDER_ETAG.fullmatch(provider_etag) is not None,
        "unannotated rollback adoption requires an exact provider script etag",
    )
    script_version = api_result(script_version_value, "Worker script version")
    require(
        isinstance(script_version, dict)
        and script_version.get("id") == expected_id,
        "Worker script version identity differs from rollback approval",
    )
    resources = script_version.get("resources")
    script = resources.get("script") if isinstance(resources, dict) else None
    require(
        isinstance(script, dict) and script.get("etag") == provider_etag,
        "unannotated rollback provider etag differs from approval",
    )
    require(
        not isinstance(annotations, dict)
        or (
            annotations.get("workers/tag") is None
            and annotations.get("workers/message") is None
        ),
        "bootstrap adoption may not bypass conflicting Worker annotations",
    )
    return "protected_unannotated_provider_etag_adoption"


def d1_identity(value: Any, expected_id: str, expected_name: str) -> None:
    database = api_result(value, "D1 identity")
    require(isinstance(database, dict), "D1 identity result is invalid")
    provider_id = database.get("uuid")
    require(
        isinstance(provider_id, str) and PROVIDER_ID.fullmatch(provider_id) is not None,
        "D1 provider identity is invalid",
    )
    require(
        provider_id.replace("-", "") == expected_id.replace("-", ""),
        "D1 provider identity differs from the protected expected ID",
    )
    require(database.get("name") == expected_name, "D1 provider name differs from the protected expected name")


def d1_row(value: Any) -> dict[str, Any]:
    require(isinstance(value, list) and len(value) == 1, "D1 authority query result count changed")
    item = value[0]
    require(isinstance(item, dict) and item.get("success") is True, "D1 authority query did not succeed")
    results = item.get("results")
    require(isinstance(results, list) and len(results) == 1 and isinstance(results[0], dict), "D1 authority query row shape changed")
    return results[0]


def validate_d1(row: dict[str, Any], phase: str) -> str:
    try:
        applied = json.loads(row["migration_names_json"])
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise AuthorityError("D1 migration authority inventory is invalid") from error
    expand = migration_inventory(EXPAND_DIR)
    contract = migration_inventory(CONTRACT_DIR)
    require(
        isinstance(applied, list) and all(isinstance(name, str) for name in applied),
        "D1 migration authority inventory is invalid",
    )
    if phase == "release-pre":
        expand_set = set(expand)
        contract_set = set(contract)
        applied_expand = [name for name in applied if name in expand_set]
        applied_contract = [name for name in applied if name in contract_set]
        require(
            len(applied) == len(set(applied))
            and set(applied) <= expand_set | contract_set
            and applied_expand == expand[: len(applied_expand)]
            and applied_contract in ([], contract),
            "pre-mutation D1 migration inventory is partial, reordered, or unknown",
        )
        if applied_contract:
            for required_expand in CONTRACT_COMPATIBILITY_FLOORS.values():
                require(
                    required_expand in applied_expand,
                    "pre-mutation D1 contract ledger lacks its required expand",
                )
        # A release may be the operation that appends the newest additive
        # expand. Classify the already-live contract state without pretending
        # that the pre-mutation ledger must yet equal the checked-in tip.
        state = "contract_applied" if applied_contract == contract else "expand_only"
    elif applied == expand:
        state = "expand_only"
    else:
        expand_set = set(expand)
        contract_set = set(contract)
        require(
            len(applied) == len(set(applied))
            and set(applied) == expand_set | contract_set
            and [name for name in applied if name in expand_set] == expand
            and [name for name in applied if name in contract_set] == contract,
            "D1 migration inventory is partial, reordered, or unknown",
        )
        # Once contracts exist, later additive expands append to the durable D1
        # ledger after them. Preserve the order within each authority directory
        # while accepting that legitimate phase interleaving.
        state = "contract_applied"
    if phase in ("pre", "post"):
        for field in ZERO_FIELDS:
            require(row.get(field) == 0, f"D1 protected drain is nonzero: {field}")
    if state == "expand_only":
        require(row.get("media_rollout_phase") == "expand", "media rollout is not in the expand phase")
        require(row.get("r2_rollout_phase") == "fenced", "R2 provider mutations are not fenced")
        require(row.get("contract_assertion_table_count") == 0, "contract assertion ledger is partially installed")
        require(row.get("contract_enforcement_trigger_count") == 0, "contract enforcement is partially installed")
    else:
        require(row.get("media_rollout_phase") == "enforced", "media contract rollout is not enforced")
        require(row.get("r2_rollout_phase") == "enabled", "R2 claim-aware rollout is not enabled")
        require(row.get("contract_assertion_table_count") == 2, "contract assertion ledger is incomplete")
        require(row.get("contract_enforcement_trigger_count") == 14, "contract enforcement trigger inventory is incomplete")
    if phase == "post":
        require(state == "contract_applied", "protected contract migrations were not durably applied")
    return state


def safe_id(value: str, label: str) -> None:
    require(PROVIDER_ID.fullmatch(value) is not None, f"{label} must be a full provider UUID")


def migration_number(value: str) -> int:
    require(re.fullmatch(r"[0-9]{4}_[a-z0-9_]+\.sql", value) is not None, "migration level is invalid")
    return int(value[:4])


def validate_contract_rollback_floor(expand_level: str, contract_level: str) -> None:
    expand_number = migration_number(expand_level)
    contract_number = migration_number(contract_level)
    for target_contract, required_expand in CONTRACT_COMPATIBILITY_FLOORS.items():
        require(
            expand_number >= migration_number(required_expand),
            f"approved rollback Worker is below the expand compatibility floor for {target_contract}",
        )
        require(
            contract_number >= migration_number(target_contract),
            f"approved rollback Worker predates the protected contract protocol for {target_contract}",
        )


def validate_release_pre(args: argparse.Namespace) -> dict[str, Any]:
    require(GIT_SHA.fullmatch(args.rollback_source_sha) is not None, "rollback source must be a full lowercase Git SHA")
    require(SHA256.fullmatch(args.rollback_worker_sha256) is not None, "rollback Worker digest is invalid")
    safe_id(args.rollback_deployment_id, "rollback deployment")
    safe_id(args.rollback_version_id, "rollback version")
    require(PROVIDER_ID.fullmatch(args.expected_d1_id) is not None, "expected D1 ID is invalid")
    require(re.fullmatch(r"[a-z][a-z0-9-]{0,62}", args.expected_d1_name) is not None, "expected D1 name is invalid")
    expand = migration_inventory(EXPAND_DIR)
    contract = migration_inventory(CONTRACT_DIR)
    require(args.rollback_expand_migration_level in expand, "approved rollback expand level is not in the checked-in inventory")
    require(args.rollback_contract_migration_level in contract, "approved rollback contract level is not in the checked-in inventory")

    deployments = deployment_inventory(read_json(args.deployments, "Worker deployments"))
    rollback = find_deployment(
        deployments, args.rollback_deployment_id, args.rollback_version_id
    )
    newest = max(deployments, key=lambda item: parse_timestamp(item.get("created_on")))
    require(
        newest.get("id") == args.rollback_deployment_id,
        "approved rollback is not the current pre-mutation production deployment",
    )
    rollback_at = parse_timestamp(rollback.get("created_on"))
    now = dt.datetime.fromtimestamp(args.now_epoch, tz=dt.timezone.utc)
    require(now >= rollback_at, "authority clock predates the rollback deployment")
    observation_seconds = int((now - rollback_at).total_seconds())
    require(
        observation_seconds >= MINIMUM_ACTIVE_AGE_SECONDS,
        "current pre-mutation Worker traffic fence has not completed its observation window",
    )
    identity_evidence = version_result(
        read_json(args.rollback_version, "rollback Worker version"),
        args.rollback_version_id,
        args.rollback_source_sha,
        args.rollback_worker_sha256,
        args.rollback_expand_migration_level,
        args.rollback_contract_migration_level,
        identity_mode=args.rollback_identity_mode,
        provider_etag=args.rollback_provider_etag,
        bootstrap_confirmation=args.rollback_bootstrap_confirmation,
        script_version_value=(
            read_json(args.rollback_script_version, "rollback Worker script version")
            if args.rollback_script_version is not None
            else None
        ),
    )
    d1_identity(
        read_json(args.d1_info, "D1 identity"),
        args.expected_d1_id,
        args.expected_d1_name,
    )
    d1_state = validate_d1(
        d1_row(read_json(args.d1_authority, "pre-mutation D1 authority")),
        args.phase,
    )
    if d1_state == "contract_applied":
        validate_contract_rollback_floor(
            args.rollback_expand_migration_level,
            args.rollback_contract_migration_level,
        )
    return {
        "schema_version": 1,
        "evidence_kind": "frame_provider_release_preflight_v1",
        "phase": "release-pre",
        "approved_current_rollback": {
            "source_git_sha": args.rollback_source_sha,
            "artifact_sha256": args.rollback_worker_sha256,
            "deployment_id": args.rollback_deployment_id,
            "version_id": args.rollback_version_id,
            "expand_migration_level": args.rollback_expand_migration_level,
            "contract_migration_level": args.rollback_contract_migration_level,
            "traffic_percentage": 100,
            "identity_evidence": identity_evidence,
        },
        "d1": {
            "expected_id_matched": True,
            "expected_name": args.expected_d1_name,
            "state": d1_state,
        },
        "current_rollback_observation_seconds": observation_seconds,
        "remote_mutation_performed": False,
        "provider_credentials_retained": False,
    }


def validate(args: argparse.Namespace) -> dict[str, Any]:
    require(GIT_SHA.fullmatch(args.active_source_sha) is not None, "active source must be a full lowercase Git SHA")
    require(GIT_SHA.fullmatch(args.rollback_source_sha) is not None, "rollback source must be a full lowercase Git SHA")
    require(args.active_source_sha != args.rollback_source_sha, "active and rollback source identities must differ")
    require(SHA256.fullmatch(args.active_worker_sha256) is not None, "active Worker digest is invalid")
    require(SHA256.fullmatch(args.rollback_worker_sha256) is not None, "rollback Worker digest is invalid")
    for label, value in (
        ("active deployment", args.active_deployment_id),
        ("active version", args.active_version_id),
        ("rollback deployment", args.rollback_deployment_id),
        ("rollback version", args.rollback_version_id),
    ):
        safe_id(value, label)
    require(args.active_deployment_id != args.rollback_deployment_id, "active and rollback deployment identities must differ")
    require(args.active_version_id != args.rollback_version_id, "active and rollback version identities must differ")
    require(PROVIDER_ID.fullmatch(args.expected_d1_id) is not None, "expected D1 ID is invalid")
    require(re.fullmatch(r"[a-z][a-z0-9-]{0,62}", args.expected_d1_name) is not None, "expected D1 name is invalid")

    expand = migration_inventory(EXPAND_DIR)
    contract = migration_inventory(CONTRACT_DIR)
    active_expand_level = expand[-1]
    active_contract_level = contract[-1]
    require(
        args.rollback_expand_migration_level in expand,
        "approved rollback expand level is not in the checked-in inventory",
    )
    require(
        args.rollback_contract_migration_level in contract,
        "approved rollback contract level is not in the checked-in inventory",
    )
    if args.phase in ("pre", "post"):
        validate_contract_rollback_floor(
            args.rollback_expand_migration_level,
            args.rollback_contract_migration_level,
        )

    deployments = deployment_inventory(read_json(args.deployments, "Worker deployments"))
    active = find_deployment(deployments, args.active_deployment_id, args.active_version_id)
    rollback = find_deployment(deployments, args.rollback_deployment_id, args.rollback_version_id)
    newest = max(deployments, key=lambda item: parse_timestamp(item.get("created_on")))
    require(newest.get("id") == args.active_deployment_id, "approved active deployment is not the current production deployment")
    active_at = parse_timestamp(active.get("created_on"))
    rollback_at = parse_timestamp(rollback.get("created_on"))
    require(rollback_at < active_at, "approved rollback is not older than the active deployment")
    now = dt.datetime.fromtimestamp(args.now_epoch, tz=dt.timezone.utc)
    require(now >= active_at, "authority clock predates the active deployment")
    if args.phase in ("pre", "post"):
        require((now - active_at).total_seconds() >= MINIMUM_ACTIVE_AGE_SECONDS, "active Worker traffic fence has not completed its observation window")

    active_identity_evidence = version_result(
        read_json(args.active_version, "active Worker version"),
        args.active_version_id,
        args.active_source_sha,
        args.active_worker_sha256,
        active_expand_level,
        active_contract_level,
    )
    rollback_identity_evidence = version_result(
        read_json(args.rollback_version, "rollback Worker version"),
        args.rollback_version_id,
        args.rollback_source_sha,
        args.rollback_worker_sha256,
        args.rollback_expand_migration_level,
        args.rollback_contract_migration_level,
        identity_mode=args.rollback_identity_mode,
        provider_etag=args.rollback_provider_etag,
        bootstrap_confirmation=args.rollback_bootstrap_confirmation,
        script_version_value=(
            read_json(args.rollback_script_version, "rollback Worker script version")
            if args.rollback_script_version is not None
            else None
        ),
    )
    d1_identity(read_json(args.d1_info, "D1 identity"), args.expected_d1_id, args.expected_d1_name)
    d1_state = validate_d1(d1_row(read_json(args.d1_authority, "D1 authority")), args.phase)
    if args.phase == "release" and d1_state == "contract_applied":
        validate_contract_rollback_floor(
            args.rollback_expand_migration_level,
            args.rollback_contract_migration_level,
        )
    return {
        "schema_version": 1,
        "evidence_kind": "frame_contract_migration_authority_v1",
        "phase": args.phase,
        "active_worker": {
            "source_git_sha": args.active_source_sha,
            "artifact_sha256": args.active_worker_sha256,
            "deployment_id": args.active_deployment_id,
            "version_id": args.active_version_id,
            "expand_migration_level": active_expand_level,
            "contract_migration_level": active_contract_level,
            "traffic_percentage": 100,
            "identity_evidence": active_identity_evidence,
        },
        "minimum_compatible_worker": {
            "source_git_sha": args.rollback_source_sha,
            "artifact_sha256": args.rollback_worker_sha256,
            "deployment_id": args.rollback_deployment_id,
            "version_id": args.rollback_version_id,
            "expand_migration_level": args.rollback_expand_migration_level,
            "contract_migration_level": args.rollback_contract_migration_level,
            "approved_rollback": True,
            "identity_evidence": rollback_identity_evidence,
        },
        "d1": {
            "expected_id_matched": True,
            "expected_name": args.expected_d1_name,
            "expand_level": migration_inventory(EXPAND_DIR)[-1],
            "contract_level": migration_inventory(CONTRACT_DIR)[-1],
            "state": d1_state,
            "protected_residuals_zero": args.phase in ("pre", "post"),
        },
        "active_observation_seconds": int((now - active_at).total_seconds()),
        "provider_credentials_retained": False,
    }


def write(path: pathlib.Path, value: dict[str, Any]) -> None:
    require(not path.is_symlink(), "evidence output may not be a symbolic link")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def self_test() -> None:
    import tempfile

    now = 2_000_000_000
    active_source = "1" * 40
    rollback_source = "2" * 40
    active_digest = "a" * 64
    rollback_digest = "b" * 64
    active_deployment = "11111111-1111-4111-8111-111111111111"
    active_version = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
    rollback_deployment = "22222222-2222-4222-8222-222222222222"
    rollback_version = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
    expand = migration_inventory(EXPAND_DIR)
    row = {
        "migration_names_json": json.dumps(expand),
        "media_rollout_phase": "expand",
        "r2_rollout_phase": "fenced",
        "contract_assertion_table_count": 0,
        "contract_enforcement_trigger_count": 0,
        **{field: 0 for field in ZERO_FIELDS},
    }
    timestamp = lambda seconds: dt.datetime.fromtimestamp(seconds, tz=dt.timezone.utc).isoformat().replace("+00:00", "Z")
    deployments = {"success": True, "errors": [], "result": {"deployments": [
        {"id": active_deployment, "created_on": timestamp(now - 1800), "versions": [{"version_id": active_version, "percentage": 100}]},
        {"id": rollback_deployment, "created_on": timestamp(now - 3600), "versions": [{"version_id": rollback_version, "percentage": 100}]},
    ]}}
    rollback_expand = expand[-2]
    rollback_contract = migration_inventory(CONTRACT_DIR)[-1]
    message = lambda source, digest, expand_level, contract_level: (
        f"frame-source:{source};frame-worker-sha256:{digest};"
        f"frame-expand:{expand_level};frame-contract:{contract_level}"
    )
    with tempfile.TemporaryDirectory(prefix="frame-contract-authority-") as directory:
        root = pathlib.Path(directory)
        values = {
            "deployments.json": deployments,
            "active.json": {"success": True, "errors": [], "result": {"id": active_version, "annotations": {"workers/tag": active_source, "workers/message": message(active_source, active_digest, expand[-1], rollback_contract)}}},
            "rollback.json": {"success": True, "errors": [], "result": {"id": rollback_version, "annotations": {"workers/tag": rollback_source, "workers/message": message(rollback_source, rollback_digest, rollback_expand, rollback_contract)}}},
            "d1.json": {"success": True, "errors": [], "result": {"uuid": "cccccccc-cccc-4ccc-8ccc-cccccccccccc", "name": "frame"}},
            "authority.json": [{"success": True, "results": [row]}],
        }
        for name, value in values.items():
            (root / name).write_text(json.dumps(value), encoding="utf-8")
        args = argparse.Namespace(
            phase="pre", deployments=root / "deployments.json", active_version=root / "active.json",
            rollback_version=root / "rollback.json", d1_info=root / "d1.json",
            d1_authority=root / "authority.json", active_source_sha=active_source,
            active_worker_sha256=active_digest, active_deployment_id=active_deployment,
            active_version_id=active_version, rollback_source_sha=rollback_source,
            rollback_worker_sha256=rollback_digest, rollback_deployment_id=rollback_deployment,
            rollback_version_id=rollback_version,
            rollback_expand_migration_level=rollback_expand,
            rollback_contract_migration_level=rollback_contract,
            rollback_identity_mode="annotated",
            rollback_provider_etag=None,
            rollback_bootstrap_confirmation=None,
            rollback_script_version=None,
            expected_d1_id="cccccccccccc4ccc8ccccccccccccccc",
            expected_d1_name="frame", now_epoch=now, evidence=root / "evidence.json",
        )
        validate(args)
        rejected = 0
        for field, value in (
            ("r2_rollout_phase", "enabled"),
            ("media_legacy_residual_count", 1),
            ("migration_names_json", json.dumps(expand[:-1])),
        ):
            original = row[field]
            row[field] = value
            (root / "authority.json").write_text(json.dumps([{"success": True, "results": [row]}]), encoding="utf-8")
            try:
                validate(args)
            except AuthorityError:
                rejected += 1
            else:
                raise AuthorityError(f"self-test accepted unsafe {field}")
            row[field] = original
        (root / "authority.json").write_text(
            json.dumps([{"success": True, "results": [row]}]), encoding="utf-8"
        )
        for filename, mutate in (
            ("d1.json", lambda value: value["result"].update(uuid="d" * 32)),
            (
                "active.json",
                lambda value: value["result"]["annotations"].update(
                    {"workers/tag": "3" * 40}
                ),
            ),
            (
                "deployments.json",
                lambda value: value["result"]["deployments"][0]["versions"][0].update(
                    percentage=99
                ),
            ),
        ):
            original = json.loads(json.dumps(values[filename]))
            mutated = json.loads(json.dumps(original))
            mutate(mutated)
            (root / filename).write_text(json.dumps(mutated), encoding="utf-8")
            try:
                validate(args)
            except AuthorityError:
                rejected += 1
            else:
                raise AuthorityError(f"self-test accepted unsafe {filename}")
            (root / filename).write_text(json.dumps(original), encoding="utf-8")

        # Model a healthy database where protected contracts were applied
        # before a later expand migration was appended to the D1 ledger.
        row["migration_names_json"] = json.dumps(
            expand[:-1] + migration_inventory(CONTRACT_DIR) + expand[-1:]
        )
        row["media_rollout_phase"] = "enforced"
        row["r2_rollout_phase"] = "enabled"
        row["contract_assertion_table_count"] = 2
        row["contract_enforcement_trigger_count"] = 14
        (root / "authority.json").write_text(
            json.dumps([{"success": True, "results": [row]}]), encoding="utf-8"
        )
        args.phase = "post"
        validate(args)

        unsafe_contract = next(
            name
            for name in migration_inventory(CONTRACT_DIR)
            if name.startswith("0032_")
        )
        release_rollback = json.loads(json.dumps(values["rollback.json"]))
        release_rollback["result"]["annotations"]["workers/message"] = message(
            rollback_source,
            rollback_digest,
            rollback_expand,
            unsafe_contract,
        )
        (root / "rollback.json").write_text(
            json.dumps(release_rollback), encoding="utf-8"
        )
        args.phase = "release"
        args.rollback_contract_migration_level = unsafe_contract
        try:
            validate(args)
        except AuthorityError:
            rejected += 1
        else:
            raise AuthorityError(
                "self-test accepted a release rollback below the live D1 contract floor"
            )
        args.phase = "post"
        args.rollback_contract_migration_level = rollback_contract
        (root / "rollback.json").write_text(
            json.dumps(values["rollback.json"]), encoding="utf-8"
        )

        for unsafe_expand, unsafe_contract in (
            (
                next(name for name in expand if name.startswith("0026_")),
                rollback_contract,
            ),
            (
                rollback_expand,
                next(
                    name
                    for name in migration_inventory(CONTRACT_DIR)
                    if name.startswith("0032_")
                ),
            ),
        ):
            try:
                validate_contract_rollback_floor(unsafe_expand, unsafe_contract)
            except AuthorityError:
                rejected += 1
            else:
                raise AuthorityError(
                    "self-test accepted a rollback below a contract compatibility floor"
                )

        bootstrap_etag = "e" * 32
        bootstrap_deployments = {
            "success": True,
            "errors": [],
            "result": {
                "deployments": [
                    {
                        "id": rollback_deployment,
                        "created_on": timestamp(now - 1200),
                        "versions": [
                            {"version_id": rollback_version, "percentage": 100}
                        ],
                    }
                ]
            },
        }
        (root / "bootstrap-deployments.json").write_text(
            json.dumps(bootstrap_deployments), encoding="utf-8"
        )
        (root / "bootstrap-version.json").write_text(
            json.dumps(
                {
                    "success": True,
                    "errors": [],
                    "result": {
                        "id": rollback_version,
                        "annotations": {},
                    },
                }
            ),
            encoding="utf-8",
        )
        (root / "bootstrap-script-version.json").write_text(
            json.dumps(
                {
                    "success": True,
                    "errors": [],
                    "result": {
                        "id": rollback_version,
                        "resources": {"script": {"etag": bootstrap_etag}},
                    },
                }
            ),
            encoding="utf-8",
        )
        bootstrap_args = argparse.Namespace(
            phase="release-pre",
            deployments=root / "bootstrap-deployments.json",
            rollback_version=root / "bootstrap-version.json",
            d1_info=root / "d1.json",
            d1_authority=root / "authority.json",
            evidence=root / "bootstrap-evidence.json",
            rollback_source_sha=rollback_source,
            rollback_worker_sha256=rollback_digest,
            rollback_deployment_id=rollback_deployment,
            rollback_version_id=rollback_version,
            rollback_expand_migration_level=rollback_expand,
            rollback_contract_migration_level=rollback_contract,
            rollback_identity_mode="protected-unannotated-bootstrap",
            rollback_provider_etag=bootstrap_etag,
            rollback_bootstrap_confirmation=ROLLBACK_BOOTSTRAP_CONFIRMATION,
            rollback_script_version=root / "bootstrap-script-version.json",
            expected_d1_id="cccccccccccc4ccc8ccccccccccccccc",
            expected_d1_name="frame",
            now_epoch=now,
        )
        validate_release_pre(bootstrap_args)
        recent_deployments = json.loads(json.dumps(bootstrap_deployments))
        recent_deployments["result"]["deployments"][0]["created_on"] = timestamp(
            now - MINIMUM_ACTIVE_AGE_SECONDS + 1
        )
        (root / "bootstrap-deployments.json").write_text(
            json.dumps(recent_deployments), encoding="utf-8"
        )
        try:
            validate_release_pre(bootstrap_args)
        except AuthorityError:
            rejected += 1
        else:
            raise AuthorityError(
                "self-test accepted a current rollback before its observation window"
            )
        (root / "bootstrap-deployments.json").write_text(
            json.dumps(bootstrap_deployments), encoding="utf-8"
        )

        bootstrap_args.rollback_contract_migration_level = unsafe_contract
        try:
            validate_release_pre(bootstrap_args)
        except AuthorityError:
            rejected += 1
        else:
            raise AuthorityError(
                "self-test accepted a preflight rollback below the live D1 contract floor"
            )
        bootstrap_args.rollback_contract_migration_level = rollback_contract
        bootstrap_args.rollback_provider_etag = "f" * 32
        try:
            validate_release_pre(bootstrap_args)
        except AuthorityError:
            rejected += 1
        else:
            raise AuthorityError(
                "self-test accepted an unapproved bootstrap provider etag"
            )
        bootstrap_args.rollback_provider_etag = bootstrap_etag
        (root / "bootstrap-script-version.json").write_text(
            json.dumps(
                {
                    "success": True,
                    "errors": [],
                    "result": {
                        "id": active_version,
                        "resources": {"script": {"etag": bootstrap_etag}},
                    },
                }
            ),
            encoding="utf-8",
        )
        try:
            validate_release_pre(bootstrap_args)
        except AuthorityError:
            rejected += 1
        else:
            raise AuthorityError(
                "self-test accepted script metadata for a different Worker version"
            )
        require(rejected == 13, "authority self-test rejection count drifted")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--phase", choices=("release-pre", "release", "pre", "post"))
    for name in (
        "deployments",
        "active-version",
        "rollback-version",
        "rollback-script-version",
        "d1-info",
        "d1-authority",
        "evidence",
    ):
        parser.add_argument(f"--{name}", type=pathlib.Path)
    for name in (
        "active-source-sha", "active-worker-sha256", "active-deployment-id", "active-version-id",
        "rollback-source-sha", "rollback-worker-sha256", "rollback-deployment-id", "rollback-version-id",
        "rollback-expand-migration-level", "rollback-contract-migration-level",
        "expected-d1-id", "expected-d1-name",
    ):
        parser.add_argument(f"--{name}")
    parser.add_argument(
        "--rollback-identity-mode",
        choices=("annotated", "protected-unannotated-bootstrap"),
        default="annotated",
    )
    parser.add_argument("--rollback-provider-etag")
    parser.add_argument("--rollback-bootstrap-confirmation")
    parser.add_argument("--now-epoch", type=int)
    args = parser.parse_args()
    if args.self_test:
        supplied = [
            value
            for key, value in vars(args).items()
            if key not in ("self_test", "rollback_identity_mode")
        ]
        if (
            any(value is not None for value in supplied)
            or args.rollback_identity_mode != "annotated"
        ):
            parser.error("--self-test cannot be combined with live arguments")
    else:
        common = (
            "phase",
            "deployments",
            "rollback_version",
            "d1_info",
            "d1_authority",
            "evidence",
            "rollback_source_sha",
            "rollback_worker_sha256",
            "rollback_deployment_id",
            "rollback_version_id",
            "rollback_expand_migration_level",
            "rollback_contract_migration_level",
            "expected_d1_id",
            "expected_d1_name",
            "now_epoch",
        )
        full = common + (
            "active_version",
            "d1_authority",
            "active_source_sha",
            "active_worker_sha256",
            "active_deployment_id",
            "active_version_id",
        )
        required = common if args.phase == "release-pre" else full
        if any(getattr(args, name) is None for name in required):
            parser.error("live authority verification requires every protected input")
        if args.rollback_identity_mode == "protected-unannotated-bootstrap":
            if (
                args.rollback_provider_etag is None
                or args.rollback_bootstrap_confirmation is None
                or args.rollback_script_version is None
            ):
                parser.error(
                    "bootstrap adoption requires provider etag and exact confirmation"
                )
        elif (
            args.rollback_provider_etag is not None
            or args.rollback_bootstrap_confirmation is not None
        ):
            parser.error("annotated rollback mode may not carry bootstrap inputs")
    return args


def main() -> int:
    args = parse_args()
    try:
        if args.self_test:
            self_test()
            print("contract migration authority self-test passed: rejected 13 unsafe provider, D1, fence, bootstrap, and compatibility cases")
            return 0
        evidence = (
            validate_release_pre(args)
            if args.phase == "release-pre"
            else validate(args)
        )
        write(args.evidence, evidence)
        print(f"contract migration authority {args.phase} proof passed; redacted evidence written")
        return 0
    except (AuthorityError, OSError, TypeError, ValueError) as error:
        print(f"contract migration authority failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
