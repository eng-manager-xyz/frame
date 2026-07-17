#!/usr/bin/env python3
"""Prove browser-action and replay batches fence current tenant/session authority."""

from __future__ import annotations

import argparse
import json
import pathlib
import sqlite3
from collections.abc import Callable


ROOT = pathlib.Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
NOW = 1_000_000
USER = "00000000-0000-7000-8000-000000000001"
ORGANIZATION = "00000000-0000-7000-8000-000000000002"
OTHER_ORGANIZATION = "00000000-0000-7000-8000-000000000012"
SESSION = "00000000-0000-7000-8000-000000000003"
FAMILY = "00000000-0000-7000-8000-000000000004"
GRANT = "00000000-0000-7000-8000-000000000005"
LAST_OPERATION = "00000000-0000-7000-8000-000000000006"
ACTION_OPERATION = "00000000-0000-7000-8000-000000000007"
SPACE = "00000000-0000-7000-8000-000000000008"
REPLAY_GRANT = "00000000-0000-7000-8000-000000000009"
REPLAY_OPERATION = "00000000-0000-7000-8000-00000000000a"
REPLAY_LAST_OPERATION = "00000000-0000-7000-8000-00000000000b"
ACTION = "organization.spaces.create.v1"


class ConformanceFailure(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ConformanceFailure(message)


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:", isolation_level=None)
    connection.execute("PRAGMA foreign_keys = ON")
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    seed(connection)
    return connection


def seed(connection: sqlite3.Connection) -> None:
    connection.execute(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms,"
        "active_organization_id,organization_preference_revision) VALUES (?,?,?,?,?,?,3)",
        (USER, "browser-owner@sqlite.invalid", "Browser Owner", NOW, NOW, ORGANIZATION),
    )
    connection.execute(
        "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,revision) "
        "VALUES (?,?,?,'active','{}',?,?,7)",
        (ORGANIZATION, USER, "Browser organization", NOW, NOW),
    )
    connection.execute(
        "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,revision) "
        "VALUES (?,?,?,'active','{}',?,?,11)",
        (OTHER_ORGANIZATION, USER, "Other organization", NOW + 1, NOW + 1),
    )
    connection.execute(
        "INSERT INTO organization_members(organization_id,user_id,role,state,created_at_ms,updated_at_ms,revision) "
        "VALUES (?,?,'owner','active',?,?,0)",
        (ORGANIZATION, USER, NOW, NOW),
    )
    connection.execute(
        "INSERT INTO organization_members(organization_id,user_id,role,state,created_at_ms,updated_at_ms,revision) "
        "VALUES (?,?,'admin','active',?,?,4)",
        (OTHER_ORGANIZATION, USER, NOW + 1, NOW + 1),
    )
    connection.execute(
        "INSERT INTO auth_identities_v2(user_id,identity_revision,session_version,created_at_ms,updated_at_ms) "
        "VALUES (?,1,0,?,?)",
        (USER, NOW, NOW),
    )
    connection.execute(
        "INSERT INTO auth_sessions_v2("
        "id,family_id,user_id,client_kind,token_key_version,token_digest,csrf_key_version,csrf_digest,"
        "browser_origin,issued_at_ms,rotated_at_ms,idle_expires_at_ms,absolute_expires_at_ms,"
        "session_version,generation,state,revision,last_operation_id) "
        "VALUES (?,?,?,'browser',1,?,1,?,'https://frame.engmanager.xyz',?,?,?,?,0,0,'active',0,?)",
        (
            SESSION,
            FAMILY,
            USER,
            "a" * 64,
            "b" * 64,
            NOW - 1_000,
            NOW - 1_000,
            NOW + 100_000,
            NOW + 200_000,
            LAST_OPERATION,
        ),
    )
    connection.execute(
        "INSERT INTO auth_session_mutation_grants_v2("
        "id,session_id,user_id,generation,token_key_version,token_digest,created_at_ms,last_operation_id) "
        "VALUES (?,?,?,0,1,?,?,?)",
        (GRANT, SESSION, USER, "a" * 64, NOW, LAST_OPERATION),
    )


def prechecked_authority(connection: sqlite3.Connection) -> tuple[str, int, int]:
    row = connection.execute(
        "SELECT m.role,m.revision,u.organization_preference_revision "
        "FROM users u JOIN organization_members m ON m.user_id=u.id "
        "AND m.organization_id=u.active_organization_id "
        "WHERE u.id=? AND m.state='active'",
        (USER,),
    ).fetchone()
    require(row is not None, "seeded membership was not admitted")
    return str(row[0]), int(row[1]), int(row[2])


def run_action_batch(
    connection: sqlite3.Connection,
    expected_role: str,
    expected_membership_revision: int,
    expected_selection_revision: int,
) -> None:
    receipt = json.dumps(
        {
            "schema_version": "frame.web-action-receipt.v1",
            "action": ACTION,
            "effect_state": "applied",
            "revision": 8,
            "invalidated": ["spaces", "workspace"],
        },
        separators=(",", ":"),
    )
    effect = json.dumps(
        {"value": "Race-fenced space", "resource_id": None},
        separators=(",", ":"),
    )
    statements: list[tuple[str, tuple[object, ...]]] = [
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'selection_authority',1,(SELECT COUNT(*) FROM users u "
            "JOIN organizations o ON o.id=u.active_organization_id AND o.status='active' "
            "WHERE u.id=? AND u.status='active' AND u.deleted_at_ms IS NULL "
            "AND u.active_organization_id=? AND u.organization_preference_revision=?))",
            (ACTION_OPERATION, USER, ORGANIZATION, expected_selection_revision),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'organization_revision',?,(SELECT revision FROM organizations "
            "WHERE id=? AND status='active'))",
            (ACTION_OPERATION, 7, ORGANIZATION),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'membership_authority',1,(SELECT COUNT(*) FROM organization_members m "
            "JOIN organizations o ON o.id=m.organization_id AND o.status='active' "
            "JOIN users u ON u.id=m.user_id AND u.status='active' AND u.deleted_at_ms IS NULL "
            "WHERE m.organization_id=? AND m.user_id=? AND m.state='active' "
            "AND m.role=? AND m.revision=?))",
            (
                ACTION_OPERATION,
                ORGANIZATION,
                USER,
                expected_role,
                expected_membership_revision,
            ),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'mutation_grant',1,(SELECT COUNT(*) FROM auth_session_mutation_grants_v2 g "
            "JOIN auth_sessions_v2 s ON s.id=g.session_id AND s.user_id=g.user_id "
            "JOIN auth_identities_v2 i ON i.user_id=g.user_id "
            "JOIN users u ON u.id=g.user_id AND u.status='active' AND u.deleted_at_ms IS NULL "
            "WHERE g.id=? AND g.session_id=? AND g.user_id=? AND s.state='active' "
            "AND s.generation=g.generation AND s.token_key_version=g.token_key_version "
            "AND s.token_digest=g.token_digest AND s.session_version=i.session_version "
            "AND s.idle_expires_at_ms>? AND s.absolute_expires_at_ms>?))",
            (ACTION_OPERATION, GRANT, SESSION, USER, NOW, NOW),
        ),
        (
            "INSERT INTO authenticated_web_action_operations_v1("
            "operation_id,organization_id,user_id,action,idempotency_key,request_digest,state,"
            "response_json,created_at_ms,completed_at_ms) "
            "VALUES (?,?,?,?,?,?,'claimed',NULL,?,NULL)",
            (ACTION_OPERATION, ORGANIZATION, USER, ACTION, "race-fence-1", "d" * 64, NOW),
        ),
        (
            "INSERT INTO spaces(id,organization_id,created_by_user_id,name,is_primary,is_public,"
            "settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision) "
            "VALUES (?,?,?,'Race-fenced space',0,0,'{}',?,?,NULL,0)",
            (SPACE, ORGANIZATION, USER, NOW, NOW),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'product_effect',1,changes())",
            (ACTION_OPERATION,),
        ),
        (
            "INSERT INTO authenticated_web_action_effects_v1("
            "operation_id,organization_id,user_id,action,effect_state,value_json,created_at_ms) "
            "VALUES (?,?,?,?,?,?,?)",
            (ACTION_OPERATION, ORGANIZATION, USER, ACTION, "applied", effect, NOW),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'action_effect',1,changes())",
            (ACTION_OPERATION,),
        ),
        (
            "UPDATE organizations SET revision=revision+1,updated_at_ms=? "
            "WHERE id=? AND revision=7 AND status='active'",
            (NOW, ORGANIZATION),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'organization_update',1,changes())",
            (ACTION_OPERATION,),
        ),
        (
            "UPDATE authenticated_web_action_operations_v1 "
            "SET state='complete',response_json=?,completed_at_ms=? "
            "WHERE operation_id=? AND state='claimed'",
            (receipt, NOW, ACTION_OPERATION),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'operation_complete',1,changes())",
            (ACTION_OPERATION,),
        ),
        (
            "DELETE FROM auth_session_mutation_grants_v2 WHERE id=? AND session_id=? AND user_id=?",
            (GRANT, SESSION, USER),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'grant_consumed',1,changes())",
            (ACTION_OPERATION,),
        ),
        (
            "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?",
            (ACTION_OPERATION,),
        ),
    ]
    connection.execute("BEGIN IMMEDIATE")
    try:
        for sql, parameters in statements:
            connection.execute(sql, parameters)
    except sqlite3.Error:
        connection.rollback()
        raise
    connection.commit()


def product_snapshot(connection: sqlite3.Connection) -> dict[str, int]:
    scalar = lambda sql: int(connection.execute(sql).fetchone()[0])
    return {
        "organization_revision": scalar(
            f"SELECT revision FROM organizations WHERE id='{ORGANIZATION}'"
        ),
        "grants": scalar("SELECT COUNT(*) FROM auth_session_mutation_grants_v2"),
        "spaces": scalar("SELECT COUNT(*) FROM spaces"),
        "operations": scalar("SELECT COUNT(*) FROM authenticated_web_action_operations_v1"),
        "effects": scalar("SELECT COUNT(*) FROM authenticated_web_action_effects_v1"),
        "assertions": scalar("SELECT COUNT(*) FROM authenticated_web_action_assertions_v1"),
    }


def denied_case(name: str, fault: Callable[[sqlite3.Connection], None]) -> dict[str, object]:
    connection = database()
    role, revision, selection_revision = prechecked_authority(connection)
    fault(connection)
    try:
        run_action_batch(connection, role, revision, selection_revision)
    except sqlite3.IntegrityError:
        pass
    else:
        raise ConformanceFailure(f"{name}: stale membership authority was accepted")
    snapshot = product_snapshot(connection)
    require(
        snapshot
        == {
            "organization_revision": 7,
            "grants": 1,
            "spaces": 0,
            "operations": 0,
            "effects": 0,
            "assertions": 0,
        },
        f"{name}: denied batch was not atomic: {snapshot}",
    )
    return {"name": name, "rolled_back": True, "snapshot": snapshot}


def success_case() -> dict[str, object]:
    connection = database()
    role, revision, selection_revision = prechecked_authority(connection)
    run_action_batch(connection, role, revision, selection_revision)
    snapshot = product_snapshot(connection)
    require(
        snapshot
        == {
            "organization_revision": 8,
            "grants": 0,
            "spaces": 1,
            "operations": 1,
            "effects": 1,
            "assertions": 0,
        },
        f"authorized batch did not commit exactly once: {snapshot}",
    )
    return {"name": "unchanged_owner_authority", "committed": True, "snapshot": snapshot}


def issue_replay_grant(connection: sqlite3.Connection) -> None:
    connection.execute(
        "INSERT INTO auth_session_mutation_grants_v2("
        "id,session_id,user_id,generation,token_key_version,token_digest,created_at_ms,last_operation_id) "
        "VALUES (?,?,?,0,1,?,?,?)",
        (REPLAY_GRANT, SESSION, USER, "a" * 64, NOW + 1, REPLAY_LAST_OPERATION),
    )


def run_replay_consumption_batch(
    connection: sqlite3.Connection,
    expected_role: str,
    expected_membership_revision: int,
    expected_selection_revision: int,
) -> None:
    stored = connection.execute(
        "SELECT response_json FROM authenticated_web_action_operations_v1 "
        "WHERE operation_id=? AND organization_id=? AND user_id=? AND action=? "
        "AND state='complete'",
        (ACTION_OPERATION, ORGANIZATION, USER, ACTION),
    ).fetchone()
    require(stored is not None and stored[0], "completed operation was unavailable for replay")
    statements: list[tuple[str, tuple[object, ...]]] = [
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'selection_authority',1,(SELECT COUNT(*) FROM users u "
            "JOIN organizations o ON o.id=u.active_organization_id AND o.status='active' "
            "WHERE u.id=? AND u.status='active' AND u.deleted_at_ms IS NULL "
            "AND u.active_organization_id=? AND u.organization_preference_revision=?))",
            (REPLAY_OPERATION, USER, ORGANIZATION, expected_selection_revision),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'membership_authority',1,(SELECT COUNT(*) FROM organization_members m "
            "JOIN organizations o ON o.id=m.organization_id AND o.status='active' "
            "JOIN users u ON u.id=m.user_id AND u.status='active' AND u.deleted_at_ms IS NULL "
            "WHERE m.organization_id=? AND m.user_id=? AND m.state='active' "
            "AND m.role=? AND m.revision=?))",
            (
                REPLAY_OPERATION,
                ORGANIZATION,
                USER,
                expected_role,
                expected_membership_revision,
            ),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'mutation_grant',1,(SELECT COUNT(*) FROM auth_session_mutation_grants_v2 g "
            "JOIN auth_sessions_v2 s ON s.id=g.session_id AND s.user_id=g.user_id "
            "JOIN auth_identities_v2 i ON i.user_id=g.user_id "
            "JOIN users u ON u.id=g.user_id AND u.status='active' AND u.deleted_at_ms IS NULL "
            "WHERE g.id=? AND g.session_id=? AND g.user_id=? AND s.state='active' "
            "AND s.generation=g.generation AND s.token_key_version=g.token_key_version "
            "AND s.token_digest=g.token_digest AND s.session_version=i.session_version "
            "AND s.idle_expires_at_ms>? AND s.absolute_expires_at_ms>?))",
            (REPLAY_OPERATION, REPLAY_GRANT, SESSION, USER, NOW, NOW),
        ),
        (
            "DELETE FROM auth_session_mutation_grants_v2 WHERE id=? AND session_id=? AND user_id=?",
            (REPLAY_GRANT, SESSION, USER),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'grant_consumed',1,changes())",
            (REPLAY_OPERATION,),
        ),
        (
            "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?",
            (REPLAY_OPERATION,),
        ),
    ]
    connection.execute("BEGIN IMMEDIATE")
    try:
        for sql, parameters in statements:
            connection.execute(sql, parameters)
    except sqlite3.Error:
        connection.rollback()
        raise
    connection.commit()


def replay_denied_case(
    name: str, fault: Callable[[sqlite3.Connection], None]
) -> dict[str, object]:
    connection = database()
    role, revision, selection_revision = prechecked_authority(connection)
    run_action_batch(connection, role, revision, selection_revision)
    issue_replay_grant(connection)
    role, membership_revision, selection_revision = prechecked_authority(connection)
    fault(connection)
    try:
        run_replay_consumption_batch(
            connection, role, membership_revision, selection_revision
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise ConformanceFailure(f"{name}: stale replay authority was accepted")
    snapshot = product_snapshot(connection)
    require(
        snapshot
        == {
            "organization_revision": 8,
            "grants": 1,
            "spaces": 1,
            "operations": 1,
            "effects": 1,
            "assertions": 0,
        },
        f"{name}: replay authority failure was not atomic: {snapshot}",
    )
    return {"name": name, "rolled_back": True, "snapshot": snapshot}


def viewer_denied_case() -> dict[str, object]:
    connection = database()
    connection.execute(
        "UPDATE organization_members SET role='viewer',revision=revision+1 "
        "WHERE organization_id=? AND user_id=?",
        (ORGANIZATION, USER),
    )
    admitted = connection.execute(
        "SELECT m.organization_id FROM organization_members m "
        "JOIN organizations o ON o.id=m.organization_id AND o.status='active' "
        "JOIN users u ON u.id=m.user_id AND u.active_organization_id=m.organization_id "
        "AND u.status='active' AND u.deleted_at_ms IS NULL "
        "WHERE m.user_id=? AND m.state='active' "
        "AND m.role IN ('owner','admin','member') LIMIT 1",
        (USER,),
    ).fetchone()
    require(admitted is None, "viewer entered the closed browser role contract")
    return {"name": "viewer_denied_before_dto", "denied": True}


def explicit_active_selection_case() -> dict[str, object]:
    connection = database()
    connection.execute(
        "UPDATE users SET active_organization_id=?,organization_preference_revision=4 "
        "WHERE id=?",
        (OTHER_ORGANIZATION, USER),
    )
    selected = connection.execute(
        "SELECT m.organization_id,m.role,u.organization_preference_revision FROM users u "
        "JOIN organization_members m ON m.user_id=u.id "
        "AND m.organization_id=u.active_organization_id AND m.state='active' "
        "AND m.role IN ('owner','admin','member') "
        "JOIN organizations o ON o.id=m.organization_id AND o.status='active' "
        "WHERE u.id=? AND u.status='active' AND u.deleted_at_ms IS NULL LIMIT 1",
        (USER,),
    ).fetchone()
    require(
        selected == (OTHER_ORGANIZATION, "admin", 4),
        f"explicit active organization was not selected: {selected}",
    )
    return {"name": "explicit_active_organization_selected", "selected": True}


def absent_selection_denied_case() -> dict[str, object]:
    connection = database()
    connection.execute(
        "UPDATE users SET active_organization_id=NULL,organization_preference_revision=4 "
        "WHERE id=?",
        (USER,),
    )
    selected = connection.execute(
        "SELECT m.organization_id FROM users u "
        "JOIN organization_members m ON m.user_id=u.id "
        "AND m.organization_id=u.active_organization_id AND m.state='active' "
        "AND m.role IN ('owner','admin','member') "
        "JOIN organizations o ON o.id=m.organization_id AND o.status='active' "
        "WHERE u.id=? AND u.status='active' AND u.deleted_at_ms IS NULL LIMIT 1",
        (USER,),
    ).fetchone()
    require(selected is None, "missing organization selection fell back to a membership")
    return {"name": "absent_active_organization_denied", "denied": True}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path)
    args = parser.parse_args()
    cases = [
        success_case(),
        explicit_active_selection_case(),
        absent_selection_denied_case(),
        viewer_denied_case(),
        denied_case(
            "selection_revision_changed_after_precheck",
            lambda connection: connection.execute(
                "UPDATE users SET organization_preference_revision="
                "organization_preference_revision+1 WHERE id=?",
                (USER,),
            ),
        ),
        denied_case(
            "active_organization_changed_after_precheck",
            lambda connection: connection.execute(
                "UPDATE users SET active_organization_id=?,organization_preference_revision="
                "organization_preference_revision+1 WHERE id=?",
                (OTHER_ORGANIZATION, USER),
            ),
        ),
        denied_case(
            "role_demoted_after_precheck",
            lambda connection: connection.execute(
                "UPDATE organization_members SET role='member',revision=revision+1 "
                "WHERE organization_id=? AND user_id=?",
                (ORGANIZATION, USER),
            ),
        ),
        denied_case(
            "membership_removed_after_precheck",
            lambda connection: connection.execute(
                "UPDATE organization_members SET state='removed',revision=revision+1 "
                "WHERE organization_id=? AND user_id=?",
                (ORGANIZATION, USER),
            ),
        ),
        denied_case(
            "membership_revision_changed_after_precheck",
            lambda connection: connection.execute(
                "UPDATE organization_members SET has_pro_seat=1,revision=revision+1 "
                "WHERE organization_id=? AND user_id=?",
                (ORGANIZATION, USER),
            ),
        ),
        denied_case(
            "session_revoked_after_validation",
            lambda connection: connection.execute(
                "UPDATE auth_sessions_v2 SET state='revoked',revoked_at_ms=?,"
                "revocation_reason='operator',revision=revision+1 WHERE id=?",
                (NOW, SESSION),
            ),
        ),
        replay_denied_case(
            "completed_replay_role_demoted_after_precheck",
            lambda connection: connection.execute(
                "UPDATE organization_members SET role='member',revision=revision+1 "
                "WHERE organization_id=? AND user_id=?",
                (ORGANIZATION, USER),
            ),
        ),
        replay_denied_case(
            "completed_replay_membership_removed_after_precheck",
            lambda connection: connection.execute(
                "UPDATE organization_members SET state='removed',revision=revision+1 "
                "WHERE organization_id=? AND user_id=?",
                (ORGANIZATION, USER),
            ),
        ),
        replay_denied_case(
            "completed_replay_selection_revision_changed_after_precheck",
            lambda connection: connection.execute(
                "UPDATE users SET organization_preference_revision="
                "organization_preference_revision+1 WHERE id=?",
                (USER,),
            ),
        ),
        replay_denied_case(
            "completed_replay_active_organization_changed_after_precheck",
            lambda connection: connection.execute(
                "UPDATE users SET active_organization_id=?,organization_preference_revision="
                "organization_preference_revision+1 WHERE id=?",
                (OTHER_ORGANIZATION, USER),
            ),
        ),
        replay_denied_case(
            "completed_replay_session_revoked_after_validation",
            lambda connection: connection.execute(
                "UPDATE auth_sessions_v2 SET state='revoked',revoked_at_ms=?,"
                "revocation_reason='operator',revision=revision+1 WHERE id=?",
                (NOW, SESSION),
            ),
        ),
    ]
    result = {
        "schema": "frame.web-authenticated-action-sqlite-conformance.v1",
        "provider": "local_sqlite",
        "atomic_membership_authority": True,
        "atomic_active_selection_authority": True,
        "completed_replay_current_authority": True,
        "case_count": len(cases),
        "cases": cases,
    }
    rendered = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ConformanceFailure, OSError, sqlite3.Error) as error:
        raise SystemExit(f"web authenticated action conformance: {error}") from error
