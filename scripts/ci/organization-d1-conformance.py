#!/usr/bin/env python3
"""Exercise organization authority through a compiled local Worker and isolated D1."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import importlib.util
import json
import os
import pathlib
import re
import secrets
import sqlite3
import subprocess
import sys
import tempfile
import time
from collections.abc import Sequence
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
CONTROL = ROOT / "apps" / "control-plane"
MIGRATIONS = CONTROL / "migrations"
CONTRACT_MIGRATIONS = CONTROL / "contract-migrations"
QUERIES = CONTROL / "queries" / "organization"
SOURCE = CONTROL / "src" / "organization_repository.rs"
SURFACE = CONTROL / "src" / "organization_repository_conformance.rs"
ROUTING = CONTROL / "src" / "routing.rs"
LIB = CONTROL / "src" / "lib.rs"
CONFORMANCE_PATH = "/__frame/local/organization-repository-conformance"
TOKEN_HEADER = "x-frame-organization-repository-conformance-token"
WRANGLER_VERSION = "4.111.0"
NOW_MS = int(time.time() * 1_000)
PLACEHOLDER = re.compile(r"\?([1-9][0-9]*)")
MIGRATION_NAME = re.compile(r"([0-9]{4})_[a-z0-9_]+\.sql")


def load_shared_harness() -> Any:
    path = ROOT / "scripts" / "ci" / "auth-d1-conformance.py"
    spec = importlib.util.spec_from_file_location("frame_auth_d1_harness", path)
    if spec is None or spec.loader is None:
        raise RuntimeError("local D1 harness could not be loaded")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    module.CONFORMANCE_PATH = CONFORMANCE_PATH
    module.TOKEN_HEADER = TOKEN_HEADER
    return module


HARNESS = load_shared_harness()
ConformanceFailure = HARNESS.ConformanceFailure


ORG_SNAPSHOT = "018f47a6-7b1c-7f55-8f39-8f8a8690e140"
ORG_INVITE = "018f47a6-7b1c-7f55-8f39-8f8a8690e141"
ORG_OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690e142"
ORG_FOLDER = "018f47a6-7b1c-7f55-8f39-8f8a8690e143"
ORG_TOMBSTONE = "018f47a6-7b1c-7f55-8f39-8f8a8690e144"
ORG_AUDIT = "018f47a6-7b1c-7f55-8f39-8f8a8690e145"
ORG_AUTHORITY_RACE = "018f47a6-7b1c-7f55-8f39-8f8a8690e146"
ORG_RETENTION = "018f47a6-7b1c-7f55-8f39-8f8a8690e147"
USER_SNAPSHOT_OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a140"
USER_INVITE_OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a141"
USER_INVITEE = "018f47a6-7b1c-7f55-8f39-8f8a8690a144"
USER_OLD_OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a142"
USER_NEW_OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a146"
USER_FOLDER_OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a143"
USER_FOLDER_MEMBER = "018f47a6-7b1c-7f55-8f39-8f8a8690a147"
USER_TOMBSTONE_OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a148"
USER_SUPPORT = "018f47a6-7b1c-7f55-8f39-8f8a8690a149"
USER_AUDIT_OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a150"
USER_AUTHORITY_OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a151"
USER_AUTHORITY_MEMBER = "018f47a6-7b1c-7f55-8f39-8f8a8690a152"
INVITE = "018f47a6-7b1c-7f55-8f39-8f8a8690f141"
SPACE_FOLDER = "018f47a6-7b1c-7f55-8f39-8f8a8690b143"
FOLDER_ROOT = "018f47a6-7b1c-7f55-8f39-8f8a8690c143"
FOLDER_CHILD = "018f47a6-7b1c-7f55-8f39-8f8a8690c144"
FOLDER_GRANDCHILD = "018f47a6-7b1c-7f55-8f39-8f8a8690c145"
SPACE_AUTHORITY_RACE = "018f47a6-7b1c-7f55-8f39-8f8a8690b146"
FOLDER_AUTHORITY_RACE = "018f47a6-7b1c-7f55-8f39-8f8a8690c146"
FOLDER_AFTER_DOWNGRADE = "018f47a6-7b1c-7f55-8f39-8f8a8690c147"
FIXTURE_OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a8690ff14"


def digest(seed: int) -> str:
    return f"{seed:064x}"


def sql_literal(value: object) -> str:
    return HARNESS.sql_literal(value)


def user_statements(user_id: str, label: str) -> list[str]:
    return [
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES "
        f"({sql_literal(user_id)},{sql_literal(label + '@organization.invalid')},NULL,{NOW_MS - 10_000},{NOW_MS - 10_000})",
        "INSERT INTO auth_identities_v2(user_id,identity_revision,session_version,created_at_ms,updated_at_ms,revision,last_operation_id) VALUES "
        f"({sql_literal(user_id)},1,0,{NOW_MS - 10_000},{NOW_MS - 10_000},0,{sql_literal(FIXTURE_OPERATION)})",
    ]


def organization_statement(organization_id: str, owner_id: str, name: str) -> str:
    return (
        "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,tombstoned_at_ms,revision,authority_version,retention_until_ms,recovered_at_ms,last_operation_id) VALUES "
        f"({sql_literal(organization_id)},{sql_literal(owner_id)},{sql_literal(name)},'active','{{}}',{NOW_MS - 5_000},{NOW_MS - 5_000},NULL,0,0,NULL,NULL,{sql_literal(FIXTURE_OPERATION)})"
    )


def membership_statement(
    organization_id: str,
    user_id: str,
    role: str,
    *,
    state: str = "active",
) -> str:
    return (
        "INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms,revision,authority_version,last_operation_id) VALUES "
        f"({sql_literal(organization_id)},{sql_literal(user_id)},{sql_literal(role)},{sql_literal(state)},0,{NOW_MS - 4_000},{NOW_MS - 4_000},0,0,{sql_literal(FIXTURE_OPERATION)})"
    )


def fixture_statements() -> list[str]:
    users = [
        (USER_SNAPSHOT_OWNER, "snapshot-owner"),
        (USER_INVITE_OWNER, "invite-owner"),
        (USER_INVITEE, "invitee"),
        (USER_OLD_OWNER, "old-owner"),
        (USER_NEW_OWNER, "new-owner"),
        (USER_FOLDER_OWNER, "folder-owner"),
        (USER_FOLDER_MEMBER, "folder-member"),
        (USER_TOMBSTONE_OWNER, "tombstone-owner"),
        (USER_SUPPORT, "support"),
        (USER_AUDIT_OWNER, "audit-owner"),
        (USER_AUTHORITY_OWNER, "authority-owner"),
        (USER_AUTHORITY_MEMBER, "authority-member"),
    ]
    statements = [statement for row in users for statement in user_statements(*row)]
    statements.extend(
        [
            organization_statement(ORG_SNAPSHOT, USER_SNAPSHOT_OWNER, "Snapshot"),
            membership_statement(ORG_SNAPSHOT, USER_SNAPSHOT_OWNER, "owner"),
            organization_statement(ORG_INVITE, USER_INVITE_OWNER, "Invite"),
            membership_statement(ORG_INVITE, USER_INVITE_OWNER, "owner"),
            organization_statement(ORG_OWNER, USER_OLD_OWNER, "Ownership"),
            membership_statement(ORG_OWNER, USER_OLD_OWNER, "owner"),
            membership_statement(ORG_OWNER, USER_NEW_OWNER, "member"),
            organization_statement(ORG_FOLDER, USER_FOLDER_OWNER, "Folders"),
            membership_statement(ORG_FOLDER, USER_FOLDER_OWNER, "owner"),
            membership_statement(ORG_FOLDER, USER_FOLDER_MEMBER, "member"),
            organization_statement(ORG_TOMBSTONE, USER_TOMBSTONE_OWNER, "Tombstone"),
            membership_statement(ORG_TOMBSTONE, USER_TOMBSTONE_OWNER, "owner"),
            organization_statement(ORG_AUDIT, USER_AUDIT_OWNER, "Audit"),
            organization_statement(ORG_AUTHORITY_RACE, USER_AUTHORITY_OWNER, "Authority race"),
            membership_statement(ORG_AUTHORITY_RACE, USER_AUTHORITY_OWNER, "owner"),
            membership_statement(ORG_AUTHORITY_RACE, USER_AUTHORITY_MEMBER, "member"),
            organization_statement(ORG_RETENTION, USER_TOMBSTONE_OWNER, "Retention"),
            membership_statement(ORG_RETENTION, USER_TOMBSTONE_OWNER, "owner"),
            "UPDATE organizations SET status='tombstoned',tombstoned_at_ms="
            f"{NOW_MS - 20_000},retention_until_ms={NOW_MS - 10_000},revision=1,authority_version=1 WHERE id={sql_literal(ORG_RETENTION)}",
            "INSERT INTO auth_identifier_digests_v2(key_version,digest,user_id,created_at_ms,last_operation_id) VALUES "
            f"(1,{sql_literal(digest(11))},{sql_literal(USER_INVITEE)},{NOW_MS - 10_000},{sql_literal(FIXTURE_OPERATION)})",
            "INSERT INTO organization_invites(id,organization_id,invited_email_digest,invited_email_key_version,invited_by_user_id,role,status,token_digest,created_at_ms,expires_at_ms,resolved_at_ms,revision,accepted_by_user_id,last_operation_id) VALUES "
            f"({sql_literal(INVITE)},{sql_literal(ORG_INVITE)},{sql_literal(digest(11))},1,{sql_literal(USER_INVITE_OWNER)},'member','pending',{sql_literal(digest(21))},{NOW_MS - 1_000},{NOW_MS + 10_000},NULL,0,NULL,{sql_literal(FIXTURE_OPERATION)})",
            "INSERT INTO spaces(id,organization_id,created_by_user_id,name,is_primary,is_public,settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision,authority_version,last_operation_id) VALUES "
            f"({sql_literal(SPACE_FOLDER)},{sql_literal(ORG_FOLDER)},{sql_literal(USER_FOLDER_OWNER)},'Primary',1,0,'{{}}',{NOW_MS - 3_000},{NOW_MS - 3_000},NULL,0,0,{sql_literal(FIXTURE_OPERATION)})",
            "INSERT INTO space_members(space_id,user_id,role,created_at_ms,updated_at_ms,state,revision,last_operation_id) VALUES "
            f"({sql_literal(SPACE_FOLDER)},{sql_literal(USER_FOLDER_MEMBER)},'manager',{NOW_MS - 2_000},{NOW_MS - 2_000},'active',0,{sql_literal(FIXTURE_OPERATION)})",
            "INSERT INTO folders(id,organization_id,space_id,parent_id,created_by_user_id,name,is_public,settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision,depth,tree_revision,last_operation_id) VALUES "
            f"({sql_literal(FOLDER_ROOT)},{sql_literal(ORG_FOLDER)},{sql_literal(SPACE_FOLDER)},NULL,{sql_literal(USER_FOLDER_MEMBER)},'Root',0,'{{}}',{NOW_MS - 2_000},{NOW_MS - 2_000},NULL,0,0,0,{sql_literal(FIXTURE_OPERATION)})",
            "INSERT INTO folders(id,organization_id,space_id,parent_id,created_by_user_id,name,is_public,settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision,depth,tree_revision,last_operation_id) VALUES "
            f"({sql_literal(FOLDER_CHILD)},{sql_literal(ORG_FOLDER)},{sql_literal(SPACE_FOLDER)},{sql_literal(FOLDER_ROOT)},{sql_literal(USER_FOLDER_MEMBER)},'Child',0,'{{}}',{NOW_MS - 2_000},{NOW_MS - 2_000},NULL,0,1,0,{sql_literal(FIXTURE_OPERATION)})",
            "INSERT INTO folders(id,organization_id,space_id,parent_id,created_by_user_id,name,is_public,settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision,depth,tree_revision,last_operation_id) VALUES "
            f"({sql_literal(FOLDER_GRANDCHILD)},{sql_literal(ORG_FOLDER)},{sql_literal(SPACE_FOLDER)},{sql_literal(FOLDER_CHILD)},{sql_literal(USER_FOLDER_MEMBER)},'Grandchild',0,'{{}}',{NOW_MS - 2_000},{NOW_MS - 2_000},NULL,0,2,0,{sql_literal(FIXTURE_OPERATION)})",
            "INSERT INTO organization_folder_closure_v1(organization_id,space_id,ancestor_id,descendant_id,distance) VALUES "
            f"({sql_literal(ORG_FOLDER)},{sql_literal(SPACE_FOLDER)},{sql_literal(FOLDER_ROOT)},{sql_literal(FOLDER_ROOT)},0),"
            f"({sql_literal(ORG_FOLDER)},{sql_literal(SPACE_FOLDER)},{sql_literal(FOLDER_CHILD)},{sql_literal(FOLDER_CHILD)},0),"
            f"({sql_literal(ORG_FOLDER)},{sql_literal(SPACE_FOLDER)},{sql_literal(FOLDER_GRANDCHILD)},{sql_literal(FOLDER_GRANDCHILD)},0),"
            f"({sql_literal(ORG_FOLDER)},{sql_literal(SPACE_FOLDER)},{sql_literal(FOLDER_ROOT)},{sql_literal(FOLDER_CHILD)},1),"
            f"({sql_literal(ORG_FOLDER)},{sql_literal(SPACE_FOLDER)},{sql_literal(FOLDER_ROOT)},{sql_literal(FOLDER_GRANDCHILD)},2),"
            f"({sql_literal(ORG_FOLDER)},{sql_literal(SPACE_FOLDER)},{sql_literal(FOLDER_CHILD)},{sql_literal(FOLDER_GRANDCHILD)},1)",
            "INSERT INTO organization_support_authorities_v1(support_actor_id,organization_id,ticket_digest,issued_at_ms,expires_at_ms,revoked_at_ms) VALUES "
            f"({sql_literal(USER_SUPPORT)},{sql_literal(ORG_AUDIT)},{sql_literal(digest(91))},{NOW_MS - 1_000},{NOW_MS + 10_000},NULL)",
            "INSERT INTO spaces(id,organization_id,created_by_user_id,name,is_primary,is_public,settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision,authority_version,last_operation_id) VALUES "
            f"({sql_literal(SPACE_AUTHORITY_RACE)},{sql_literal(ORG_AUTHORITY_RACE)},{sql_literal(USER_AUTHORITY_OWNER)},'Authority',1,0,'{{}}',{NOW_MS - 3_000},{NOW_MS - 3_000},NULL,0,0,{sql_literal(FIXTURE_OPERATION)})",
            "INSERT INTO space_members(space_id,user_id,role,created_at_ms,updated_at_ms,state,revision,last_operation_id) VALUES "
            f"({sql_literal(SPACE_AUTHORITY_RACE)},{sql_literal(USER_AUTHORITY_MEMBER)},'manager',{NOW_MS - 2_000},{NOW_MS - 2_000},'active',0,{sql_literal(FIXTURE_OPERATION)})",
        ]
    )
    return statements


def migration_files() -> list[pathlib.Path]:
    files = sorted(MIGRATIONS.glob("*.sql"))
    contract_files = sorted(CONTRACT_MIGRATIONS.glob("*.sql"))
    if not files or not contract_files:
        raise ConformanceFailure("migration authority inventory is incomplete")
    expand_numbers: list[int] = []
    contract_numbers: list[int] = []
    for path, numbers, phase in (
        *((path, expand_numbers, "expand") for path in files),
        *((path, contract_numbers, "contract") for path in contract_files),
    ):
        match = MIGRATION_NAME.fullmatch(path.name)
        if match is None:
            raise ConformanceFailure(f"invalid {phase} migration filename: {path.name}")
        numbers.append(int(match.group(1)))
    if expand_numbers != sorted(expand_numbers):
        raise ConformanceFailure("expand migration sequence is reordered")
    if contract_numbers != sorted(contract_numbers):
        raise ConformanceFailure("contract migration sequence is reordered")
    combined = expand_numbers + contract_numbers
    if len(combined) != len(set(combined)):
        raise ConformanceFailure("combined migration authority has a duplicate number")
    if sorted(combined) != list(range(1, len(combined) + 1)):
        raise ConformanceFailure("combined migration authority is not globally contiguous")
    return files


def compile_checked_in_sql() -> None:
    database = sqlite3.connect(":memory:")
    try:
        database.execute("PRAGMA foreign_keys = ON")
        for path in migration_files():
            database.executescript(path.read_text(encoding="utf-8"))
        queries = sorted(QUERIES.glob("*.sql"))
        if len(queries) < 50:
            raise ConformanceFailure("organization query inventory is incomplete")
        for path in queries:
            sql = path.read_text(encoding="utf-8").strip()
            indexes = [int(match) for match in PLACEHOLDER.findall(sql)]
            if not indexes:
                raise ConformanceFailure("organization query has no bound contract")
            database.execute("EXPLAIN " + sql, [None] * max(indexes)).fetchall()
    except sqlite3.Error as error:
        raise ConformanceFailure("checked-in organization SQL did not compile") from error
    finally:
        database.close()


def verify_compiled_surface() -> None:
    source = SOURCE.read_text(encoding="utf-8")
    surface = "\n".join(path.read_text(encoding="utf-8") for path in (SURFACE, ROUTING, LIB))
    for forbidden in ("todo!", "unimplemented!", "organization_repository_conformance_unavailable"):
        if forbidden in source.lower() or forbidden in surface.lower():
            raise ConformanceFailure("organization repository contains an unfinished path")
    for marker in (
        CONFORMANCE_PATH,
        "FRAME_ORGANIZATION_REPOSITORY_CONFORMANCE_TOKEN",
        "Route::LocalOrganizationRepositoryConformance",
        "config.production() || !valid_repository_conformance_target",
    ):
        if marker not in surface:
            raise ConformanceFailure("compiled organization conformance surface drifted")
    if ".database\n            .batch(statements)\n            .into_send()\n            .await" not in source:
        raise ConformanceFailure("organization mutation promise no longer settles")
    if "Delay::from" in source:
        raise ConformanceFailure("organization mutations acquired a local deadline")


class WorkerServer(HARNESS.WorkerServer):
    def request(
        self,
        scenario: str,
        *,
        token: str | None = None,
        path: str = CONFORMANCE_PATH,
        host: str | None = None,
        timeout: float = 30,
    ) -> tuple[int, dict[str, Any]]:
        return super().request(
            scenario,
            token=token,
            path=path,
            host=host,
            timeout=timeout,
        )

    def start(self) -> None:
        self.log_file = self.log_path.open("w", encoding="utf-8")
        self.process = subprocess.Popen(
            [
                *self.d1.command,
                "dev",
                "--local",
                "--persist-to",
                str(self.d1.state),
                "--config",
                str(HARNESS.CONFIG),
                "--ip",
                "127.0.0.1",
                "--port",
                str(self.port),
                "--var",
                f"FRAME_ORGANIZATION_REPOSITORY_CONFORMANCE_TOKEN:{self.token}",
            ],
            cwd=ROOT,
            env=self.d1.environment,
            stdin=subprocess.DEVNULL,
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
            text=True,
        )
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise ConformanceFailure("local organization Worker exited before ready")
            try:
                import http.client

                connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=1)
                connection.request("GET", "/health")
                response = connection.getresponse()
                response.read()
                connection.close()
                return
            except OSError:
                time.sleep(0.2)
        raise ConformanceFailure("local organization Worker did not become ready")


def expect_scenario(server: WorkerServer, scenario: str) -> dict[str, Any]:
    status, payload = server.request(scenario)
    details = payload.get("details")
    if (
        status != 200
        or payload.get("outcome") != "ok"
        or not isinstance(details, dict)
        or details.get("scenario") != scenario
        or not isinstance(details.get("values"), dict)
    ):
        outcome = payload.get("outcome")
        safe_outcome = outcome if isinstance(outcome, str) and re.fullmatch(r"[a-z_]+", outcome) else "invalid"
        raise ConformanceFailure(
            f"organization scenario failed: {scenario}:{status}:{safe_outcome}"
        )
    return dict(details["values"])


def result_code(values: dict[str, Any]) -> str:
    value = values.get("result")
    if isinstance(value, str):
        return value
    error = values.get("error")
    return f"error:{error}" if isinstance(error, str) else "invalid"


def exercise_worker(server: WorkerServer) -> None:
    status, _ = server.request("snapshot_boundary", token=secrets.token_hex(32))
    if status != 404:
        raise ConformanceFailure("organization token did not fail closed")
    status, _ = server.request("snapshot_boundary", path=CONFORMANCE_PATH + "/")
    if status != 404:
        raise ConformanceFailure("organization path was not exact")
    status, _ = server.request("snapshot_boundary", host=f"localhost:{server.port}")
    if status != 404:
        raise ConformanceFailure("organization route accepted a non-exact loopback host")

    boundary = expect_scenario(server, "snapshot_boundary")
    if boundary != {
        "role": "owner",
        "status": "active",
        "cross_tenant": "access_denied",
        "unknown": "access_denied",
        "indistinguishable": True,
    }:
        safe = json.dumps(boundary, sort_keys=True, separators=(",", ":"))
        raise ConformanceFailure(f"organization boundary result changed: {safe}")
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
        futures = [
            executor.submit(expect_scenario, server, "invite_accept_a"),
            executor.submit(expect_scenario, server, "invite_accept_b"),
        ]
        invite_results = [future.result(timeout=35) for future in futures]
    if sorted(result_code(value) for value in invite_results) != [
        "accepted",
        "error:stale_authority",
    ]:
        raise ConformanceFailure("concurrent invite acceptance did not yield one winner")
    replay = expect_scenario(server, "invite_replay_and_mismatch")
    replay_values = [replay["replay_a"], replay["replay_b"]]
    mismatch_values = [replay["mismatch_a"], replay["mismatch_b"]]
    if sorted(result_code(value) for value in replay_values) != [
        "accepted",
        "error:stale_authority",
    ] or sorted(result_code(value) for value in mismatch_values) != [
        "error:conflict",
        "error:stale_authority",
    ]:
        raise ConformanceFailure("invite replay or mismatch semantics changed")
    if sum(value.get("replayed") is True for value in replay_values) != 1:
        raise ConformanceFailure("invite replay was not reconstructed from its receipt")

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
        futures = [
            executor.submit(expect_scenario, server, "ownership_transfer"),
            executor.submit(expect_scenario, server, "owner_target_removal"),
        ]
        ownership_results = [future.result(timeout=35) for future in futures]
    if sorted(result_code(value) for value in ownership_results) != [
        "applied",
        "error:stale_authority",
    ]:
        raise ConformanceFailure("ownership transfer race did not serialize")
    invariant = expect_scenario(server, "ownership_invariant")
    if invariant.get("active_owner_count") != 1 or invariant.get("pointer_owner_count") != 1:
        raise ConformanceFailure("organization owner invariant changed")

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
        futures = [
            executor.submit(expect_scenario, server, "authority_member_downgrade"),
            executor.submit(expect_scenario, server, "authority_folder_create"),
        ]
        authority_results = [future.result(timeout=35) for future in futures]
    authority_codes = sorted(result_code(value) for value in authority_results)
    if authority_codes not in (
        ["applied", "created"],
        ["applied", "error:stale_authority"],
    ):
        raise ConformanceFailure("member downgrade and write race did not serialize")
    post_downgrade = expect_scenario(server, "authority_post_downgrade")
    if post_downgrade != {"error": "stale_authority"}:
        raise ConformanceFailure("pre-downgrade authority survived its session fence")
    authority = expect_scenario(server, "authority_invariant")
    if (
        authority.get("role") != "viewer"
        or authority.get("state") != "active"
        or authority.get("session_version") != 1
        or authority.get("race_folders") not in (0, 1)
        or authority.get("post_downgrade_folders") != 0
    ):
        raise ConformanceFailure("member downgrade authority invariant changed")

    cycle = expect_scenario(server, "folder_cycle")
    if cycle != {"cycle": "stale_authority"}:
        raise ConformanceFailure("folder cycle was not rejected")
    move = expect_scenario(server, "folder_move")
    if move != {"result": "applied", "depth": 0, "self_edges": 1, "cycle_edges": 0}:
        raise ConformanceFailure("folder move invariant changed")
    tombstone = expect_scenario(server, "tombstone_lifecycle")
    if tombstone != {
        "tombstone": "tombstoned",
        "stale_old_fence": "stale_authority",
        "recover": "recovered",
    }:
        raise ConformanceFailure("tombstone recovery result changed")
    retention = expect_scenario(server, "retention_expiry")
    if retention != {
        "tombstone": "seeded_expired",
        "expired_recovery": "retention_locked",
    }:
        raise ConformanceFailure("expired recovery did not remain retention locked")
    repair = expect_scenario(server, "audit_repair")
    if (
        repair.get("finding_count", 0) < 1
        or repair.get("plan_steps", 0) < 1
        or repair.get("dry_run") is not True
        or repair.get("automatic_mutations") != 0
    ):
        safe = json.dumps(repair, sort_keys=True, separators=(",", ":"))
        raise ConformanceFailure(f"dry-run repair result changed: {safe}")
    denied_audit = expect_scenario(server, "audit_denied")
    if denied_audit != {"error": "access_denied", "tenant_audit_rows_added": 0}:
        raise ConformanceFailure("invalid support authority did not fail closed")


def assert_final_state(d1: Any) -> None:
    rows = d1.query(
        "SELECT "
        f"(SELECT status FROM organization_invites WHERE id={sql_literal(INVITE)}) AS invite_status,"
        f"(SELECT COUNT(*) FROM organization_members WHERE organization_id={sql_literal(ORG_INVITE)} AND user_id={sql_literal(USER_INVITEE)} AND state='active') AS invite_memberships,"
        f"(SELECT COUNT(*) FROM organization_members WHERE organization_id={sql_literal(ORG_OWNER)} AND role='owner' AND state='active') AS active_owners,"
        f"(SELECT COUNT(*) FROM organizations o JOIN organization_members m ON m.organization_id=o.id AND m.user_id=o.owner_id AND m.role='owner' AND m.state='active' WHERE o.id={sql_literal(ORG_OWNER)}) AS pointer_owners,"
        f"(SELECT status FROM organizations WHERE id={sql_literal(ORG_TOMBSTONE)}) AS tombstone_status,"
        f"(SELECT status FROM organizations WHERE id={sql_literal(ORG_RETENTION)}) AS retention_status,"
        f"(SELECT role FROM organization_members WHERE organization_id={sql_literal(ORG_AUTHORITY_RACE)} AND user_id={sql_literal(USER_AUTHORITY_MEMBER)}) AS authority_role,"
        f"(SELECT session_version FROM auth_identities_v2 WHERE user_id={sql_literal(USER_AUTHORITY_MEMBER)}) AS authority_session_version,"
        f"(SELECT COUNT(*) FROM folders WHERE id={sql_literal(FOLDER_AUTHORITY_RACE)} AND organization_id={sql_literal(ORG_AUTHORITY_RACE)}) AS authority_race_folders,"
        f"(SELECT COUNT(*) FROM folders WHERE id={sql_literal(FOLDER_AFTER_DOWNGRADE)} AND organization_id={sql_literal(ORG_AUTHORITY_RACE)}) AS authority_post_folders,"
        f"(SELECT COUNT(*) FROM organization_repair_plans_v1 WHERE organization_id={sql_literal(ORG_AUDIT)} AND dry_run=1) AS repair_plans,"
        f"(SELECT COUNT(*) FROM organization_members WHERE organization_id={sql_literal(ORG_AUDIT)} AND role='owner' AND state='active') AS audit_owner_mutations,"
        "(SELECT COUNT(*) FROM organization_repository_operations_v1) AS operation_count,"
        "(SELECT COUNT(*) FROM organization_audit_events_v1 WHERE outcome='allow') AS audit_allows,"
        "(SELECT COUNT(*) FROM organization_audit_events_v1 WHERE outcome='deny') AS audit_denies",
        command_class="organization_final_state",
    )
    if len(rows) != 1:
        raise ConformanceFailure("organization final state shape changed")
    row = rows[0]
    if (
        row.get("invite_status") != "accepted"
        or row.get("invite_memberships") != 1
        or row.get("active_owners") != 1
        or row.get("pointer_owners") != 1
        or row.get("tombstone_status") != "active"
        or row.get("retention_status") != "tombstoned"
        or row.get("authority_role") != "viewer"
        or row.get("authority_session_version") != 1
        or row.get("authority_race_folders") not in (0, 1)
        or row.get("authority_post_folders") != 0
        or row.get("repair_plans") != 1
        or row.get("audit_owner_mutations") != 0
        or int(row.get("operation_count", 0)) < 6
        or int(row.get("audit_allows", 0)) < 6
        or int(row.get("audit_denies", 0)) != 0
    ):
        raise ConformanceFailure("organization final invariants changed")


def parse_telemetry(log_path: pathlib.Path, token: str) -> list[dict[str, Any]]:
    clean = HARNESS.ANSI.sub("", log_path.read_text(encoding="utf-8"))
    for forbidden in (token, digest(11), digest(21), digest(91), "@organization.invalid"):
        if forbidden in clean:
            raise ConformanceFailure("organization Worker log exposed sensitive state")
    records: list[dict[str, Any]] = []
    decoder = json.JSONDecoder()
    for line in clean.splitlines():
        marker = '"event":"d1_organization_repository"'
        cursor = 0
        while (marker_at := line.find(marker, cursor)) >= 0:
            start = line.rfind("{", cursor, marker_at + 1)
            if start < 0:
                cursor = marker_at + len(marker)
                continue
            try:
                record, consumed = decoder.raw_decode(line[start:])
            except json.JSONDecodeError:
                cursor = marker_at + len(marker)
                continue
            if isinstance(record, dict) and record.get("event") == "d1_organization_repository":
                records.append(record)
            cursor = start + consumed
    allowed_fields = {"event", "operation", "outcome", "rows"}
    if not records or any(set(record) != allowed_fields for record in records):
        raise ConformanceFailure("organization telemetry shape changed")
    return records


def artifact_digest(files: Sequence[pathlib.Path]) -> str:
    value = hashlib.sha256()
    for path in files:
        value.update(path.name.encode())
        value.update(b"\0")
        value.update(path.read_bytes())
        value.update(b"\0")
    return value.hexdigest()


def write_evidence(path: pathlib.Path, telemetry: Sequence[dict[str, Any]]) -> None:
    queries = sorted(QUERIES.glob("*.sql"))
    report = {
        "schema_version": 1,
        "suite": "frame-d1-organization-repository-conformance",
        "runtime_boundary": "compiled_rust_wasm_worker_over_exact_loopback_http",
        "database": "isolated_local_wrangler_d1",
        "wrangler_version": WRANGLER_VERSION,
        "migration_count": len(migration_files()),
        "migration_digest_sha256": artifact_digest(migration_files()),
        "query_count": len(queries),
        "query_digest_sha256": artifact_digest(queries),
        "scenarios": [
            "tenant_boundary_unknown_id_indistinguishability",
            "concurrent_single_use_hashed_invite_acceptance_and_receipt_replay",
            "ownership_transfer_vs_removal_exactly_one_owner",
            "member_downgrade_vs_write_session_fence_serialization",
            "folder_cycle_rejection_and_closure_fenced_move",
            "tombstone_stale_authority_and_retention_bounded_recovery",
            "expired_recovery_retention_lock",
            "authenticated_support_graph_audit_and_dry_run_only_repair",
            "invalid_support_authority_indistinguishable_denial",
            "immutable_allow_audit_denial_nonmutation_and_redacted_telemetry",
        ],
        "telemetry_record_count": len(telemetry),
        "result": "pass",
        "not_claimed": [
            "legacy_cap_fixture_parity_or_shadow_evaluation",
            "remote_d1_contention_or_replication",
            "customer_approved_retention_window",
            "browser_or_public_api_surface",
            "provider_oauth_or_email_delivery",
            "production_owner_or_security_signoff",
        ],
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wrangler-bin")
    parser.add_argument(
        "--evidence",
        type=pathlib.Path,
        default=ROOT / "target" / "evidence" / "organization-d1-conformance.json",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        HARNESS.refuse_external_authority()
        compile_checked_in_sql()
        verify_compiled_surface()
        wrangler = HARNESS.detect_wrangler(arguments.wrangler_bin)
        with tempfile.TemporaryDirectory(prefix="frame-organization-d1-conformance-") as directory:
            root = pathlib.Path(directory)
            state = root / "state"
            state.mkdir(mode=0o700)
            d1 = HARNESS.WranglerD1(wrangler, state)
            d1.migrate()
            fixture = root / "fixture.sql"
            fixture.write_text(";\n".join(fixture_statements()) + ";\n", encoding="utf-8")
            d1.execute_file(fixture)
            token = secrets.token_hex(32)
            server = WorkerServer(d1, token, root)
            try:
                server.start()
                try:
                    exercise_worker(server)
                except ConformanceFailure:
                    if os.environ.get("FRAME_CONFORMANCE_DEBUG") == "1" and server.log_path.exists():
                        clean = HARNESS.ANSI.sub("", server.log_path.read_text(encoding="utf-8"))
                        clean = clean.replace(token, "[redacted]")
                        print("\n".join(clean.splitlines()[-80:]), file=sys.stderr)
                    raise
            finally:
                server.stop()
            assert_final_state(d1)
            telemetry = parse_telemetry(server.log_path, token)
        write_evidence(arguments.evidence.resolve(), telemetry)
    except (
        ConformanceFailure,
        OSError,
        sqlite3.Error,
        subprocess.SubprocessError,
        ValueError,
    ) as error:
        print(f"D1 organization repository conformance failed: {error}", file=sys.stderr)
        return 1
    print(
        "D1 organization repository conformance passed through compiled Worker "
        f"({len(migration_files())} migrations; Wrangler {WRANGLER_VERSION})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
