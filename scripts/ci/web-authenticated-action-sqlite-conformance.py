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
SELECTION_OPERATION = "00000000-0000-7000-8000-00000000000c"
SELECTION_REPLAY_ASSERTION = "00000000-0000-7000-8000-00000000000d"
SELECTION_REUSE_ASSERTION = "00000000-0000-7000-8000-00000000000e"
SELECTION_CROSS_TARGET_ASSERTION = "00000000-0000-7000-8000-00000000000f"
ACTION = "organization.spaces.create.v1"
SELECTION_ACTION = "organization.active-selection.update.v1"
SELECTION_IDEMPOTENCY_KEY = "active-selection-1"
SELECTION_REQUEST_DIGEST = "e" * 64


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


def active_tenant(connection: sqlite3.Connection) -> tuple[str, str, int] | None:
    row = connection.execute(
        "SELECT m.organization_id,m.role,u.organization_preference_revision FROM users u "
        "JOIN organization_members m ON m.user_id=u.id "
        "AND m.organization_id=u.active_organization_id AND m.state='active' "
        "AND m.role IN ('owner','admin','member') "
        "JOIN organizations o ON o.id=m.organization_id AND o.status='active' "
        "WHERE u.id=? AND u.status='active' AND u.deleted_at_ms IS NULL LIMIT 1",
        (USER,),
    ).fetchone()
    if row is None:
        return None
    return str(row[0]), str(row[1]), int(row[2])


def recovery_choices(connection: sqlite3.Connection) -> list[str]:
    return [
        str(row[0])
        for row in connection.execute(
            "SELECT o.id FROM organization_members m "
            "JOIN organizations o ON o.id=m.organization_id AND o.status='active' "
            "JOIN users u ON u.id=m.user_id AND u.status='active' "
            "AND u.deleted_at_ms IS NULL "
            "WHERE m.user_id=? AND m.state='active' "
            "AND m.role IN ('owner','admin','member') ORDER BY o.id",
            (USER,),
        ).fetchall()
    ]


def run_selection_action_batch(
    connection: sqlite3.Connection,
    *,
    current_organization_id: str | None,
    expected_selection_revision: int,
    grant_id: str = GRANT,
    operation_id: str = SELECTION_OPERATION,
) -> None:
    next_revision = expected_selection_revision + 1
    receipt = json.dumps(
        {
            "schema_version": "frame.web-action-receipt.v1",
            "action": SELECTION_ACTION,
            "effect_state": "applied",
            "revision": next_revision,
            "invalidated": ["dashboard"],
        },
        separators=(",", ":"),
    )
    effect = json.dumps(
        {"organization_id": OTHER_ORGANIZATION}, separators=(",", ":")
    )
    statements: list[tuple[str, tuple[object, ...]]] = [
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'selection_authority',1,(SELECT COUNT(*) FROM users "
            "WHERE id=? AND status='active' AND deleted_at_ms IS NULL "
            "AND organization_preference_revision=? "
            "AND COALESCE(active_organization_id,'')=?))",
            (
                operation_id,
                USER,
                expected_selection_revision,
                current_organization_id or "",
            ),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'membership_authority',1,(SELECT COUNT(*) "
            "FROM organization_members m "
            "JOIN organizations o ON o.id=m.organization_id AND o.status='active' "
            "JOIN users u ON u.id=m.user_id AND u.status='active' "
            "AND u.deleted_at_ms IS NULL "
            "WHERE m.organization_id=? AND m.user_id=? AND m.state='active' "
            "AND m.role IN ('owner','admin','member')))",
            (operation_id, OTHER_ORGANIZATION, USER),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'mutation_grant',1,(SELECT COUNT(*) "
            "FROM auth_session_mutation_grants_v2 g "
            "JOIN auth_sessions_v2 s ON s.id=g.session_id AND s.user_id=g.user_id "
            "JOIN auth_identities_v2 i ON i.user_id=g.user_id "
            "JOIN users u ON u.id=g.user_id AND u.status='active' "
            "AND u.deleted_at_ms IS NULL "
            "WHERE g.id=? AND g.session_id=? AND g.user_id=? AND s.state='active' "
            "AND s.generation=g.generation "
            "AND s.token_key_version=g.token_key_version "
            "AND s.token_digest=g.token_digest "
            "AND s.session_version=i.session_version "
            "AND s.idle_expires_at_ms>? AND s.absolute_expires_at_ms>?))",
            (operation_id, grant_id, SESSION, USER, NOW, NOW),
        ),
        (
            "INSERT INTO authenticated_web_action_operations_v1("
            "operation_id,organization_id,user_id,action,idempotency_key,request_digest,"
            "state,response_json,created_at_ms,completed_at_ms) "
            "VALUES (?,?,?,?,?,?,'claimed',NULL,?,NULL)",
            (
                operation_id,
                OTHER_ORGANIZATION,
                USER,
                SELECTION_ACTION,
                SELECTION_IDEMPOTENCY_KEY,
                SELECTION_REQUEST_DIGEST,
                NOW,
            ),
        ),
        (
            "UPDATE users SET active_organization_id=?,"
            "organization_preference_revision=organization_preference_revision+1,"
            "organization_last_operation_id=?,updated_at_ms=? "
            "WHERE id=? AND status='active' AND deleted_at_ms IS NULL "
            "AND organization_preference_revision=? "
            "AND COALESCE(active_organization_id,'')=?",
            (
                OTHER_ORGANIZATION,
                operation_id,
                NOW,
                USER,
                expected_selection_revision,
                current_organization_id or "",
            ),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'product_effect',1,changes())",
            (operation_id,),
        ),
        (
            "INSERT INTO authenticated_web_action_effects_v1("
            "operation_id,organization_id,user_id,action,effect_state,value_json,created_at_ms) "
            "VALUES (?,?,?,?,?,?,?)",
            (
                operation_id,
                OTHER_ORGANIZATION,
                USER,
                SELECTION_ACTION,
                "applied",
                effect,
                NOW,
            ),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'action_effect',1,changes())",
            (operation_id,),
        ),
        (
            "UPDATE authenticated_web_action_operations_v1 "
            "SET state='complete',response_json=?,completed_at_ms=? "
            "WHERE operation_id=? AND state='claimed'",
            (receipt, NOW, operation_id),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'operation_complete',1,changes())",
            (operation_id,),
        ),
        (
            "DELETE FROM auth_session_mutation_grants_v2 "
            "WHERE id=? AND session_id=? AND user_id=?",
            (grant_id, SESSION, USER),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'grant_consumed',1,changes())",
            (operation_id,),
        ),
        (
            "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?",
            (operation_id,),
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


def run_selection_replay_consumption_batch(
    connection: sqlite3.Connection,
    *,
    grant_id: str,
    assertion_operation_id: str,
    expected_selection_revision: int,
) -> None:
    statements: list[tuple[str, tuple[object, ...]]] = [
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'selection_authority',1,(SELECT COUNT(*) FROM users "
            "WHERE id=? AND status='active' AND deleted_at_ms IS NULL "
            "AND active_organization_id=? "
            "AND organization_preference_revision=?))",
            (
                assertion_operation_id,
                USER,
                OTHER_ORGANIZATION,
                expected_selection_revision,
            ),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'membership_authority',1,(SELECT COUNT(*) "
            "FROM organization_members m "
            "JOIN organizations o ON o.id=m.organization_id AND o.status='active' "
            "JOIN users u ON u.id=m.user_id AND u.status='active' "
            "AND u.deleted_at_ms IS NULL "
            "WHERE m.organization_id=? AND m.user_id=? AND m.state='active' "
            "AND m.role IN ('owner','admin','member')))",
            (assertion_operation_id, OTHER_ORGANIZATION, USER),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'mutation_grant',1,(SELECT COUNT(*) "
            "FROM auth_session_mutation_grants_v2 g "
            "JOIN auth_sessions_v2 s ON s.id=g.session_id AND s.user_id=g.user_id "
            "JOIN auth_identities_v2 i ON i.user_id=g.user_id "
            "JOIN users u ON u.id=g.user_id AND u.status='active' "
            "AND u.deleted_at_ms IS NULL "
            "WHERE g.id=? AND g.session_id=? AND g.user_id=? AND s.state='active' "
            "AND s.generation=g.generation "
            "AND s.token_key_version=g.token_key_version "
            "AND s.token_digest=g.token_digest "
            "AND s.session_version=i.session_version "
            "AND s.idle_expires_at_ms>? AND s.absolute_expires_at_ms>?))",
            (assertion_operation_id, grant_id, SESSION, USER, NOW, NOW),
        ),
        (
            "DELETE FROM auth_session_mutation_grants_v2 "
            "WHERE id=? AND session_id=? AND user_id=?",
            (grant_id, SESSION, USER),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'grant_consumed',1,changes())",
            (assertion_operation_id,),
        ),
        (
            "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?",
            (assertion_operation_id,),
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


def run_session_grant_consumption_batch(
    connection: sqlite3.Connection,
    *,
    grant_id: str,
    assertion_operation_id: str,
) -> None:
    statements: list[tuple[str, tuple[object, ...]]] = [
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'mutation_grant',1,(SELECT COUNT(*) "
            "FROM auth_session_mutation_grants_v2 g "
            "JOIN auth_sessions_v2 s ON s.id=g.session_id AND s.user_id=g.user_id "
            "JOIN auth_identities_v2 i ON i.user_id=g.user_id "
            "JOIN users u ON u.id=g.user_id AND u.status='active' "
            "AND u.deleted_at_ms IS NULL "
            "WHERE g.id=? AND g.session_id=? AND g.user_id=? AND s.state='active' "
            "AND s.generation=g.generation "
            "AND s.token_key_version=g.token_key_version "
            "AND s.token_digest=g.token_digest "
            "AND s.session_version=i.session_version "
            "AND s.idle_expires_at_ms>? AND s.absolute_expires_at_ms>?))",
            (assertion_operation_id, grant_id, SESSION, USER, NOW, NOW),
        ),
        (
            "DELETE FROM auth_session_mutation_grants_v2 "
            "WHERE id=? AND session_id=? AND user_id=?",
            (grant_id, SESSION, USER),
        ),
        (
            "INSERT INTO authenticated_web_action_assertions_v1("
            "operation_id,assertion_kind,expected_count,actual_count) "
            "VALUES (?,'grant_consumed',1,changes())",
            (assertion_operation_id,),
        ),
        (
            "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?",
            (assertion_operation_id,),
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


def selection_snapshot(connection: sqlite3.Connection) -> dict[str, object]:
    selection = connection.execute(
        "SELECT active_organization_id,organization_preference_revision,"
        "organization_last_operation_id FROM users WHERE id=?",
        (USER,),
    ).fetchone()
    require(selection is not None, "selection user disappeared")
    return {
        "active_organization_id": selection[0],
        "selection_revision": int(selection[1]),
        "last_operation_id": selection[2],
        "grants": int(
            connection.execute(
                "SELECT COUNT(*) FROM auth_session_mutation_grants_v2"
            ).fetchone()[0]
        ),
        "operations": int(
            connection.execute(
                "SELECT COUNT(*) FROM authenticated_web_action_operations_v1"
            ).fetchone()[0]
        ),
        "effects": int(
            connection.execute(
                "SELECT COUNT(*) FROM authenticated_web_action_effects_v1"
            ).fetchone()[0]
        ),
        "assertions": int(
            connection.execute(
                "SELECT COUNT(*) FROM authenticated_web_action_assertions_v1"
            ).fetchone()[0]
        ),
    }


def selection_success_snapshot(selection_revision: int) -> dict[str, object]:
    return {
        "active_organization_id": OTHER_ORGANIZATION,
        "selection_revision": selection_revision,
        "last_operation_id": SELECTION_OPERATION,
        "grants": 0,
        "operations": 1,
        "effects": 1,
        "assertions": 0,
    }


def selection_denied_snapshot(
    active_organization_id: str | None = ORGANIZATION,
    selection_revision: int = 3,
) -> dict[str, object]:
    return {
        "active_organization_id": active_organization_id,
        "selection_revision": selection_revision,
        "last_operation_id": None,
        "grants": 1,
        "operations": 0,
        "effects": 0,
        "assertions": 0,
    }


def absent_selection_recovery_case() -> dict[str, object]:
    connection = database()
    connection.execute(
        "UPDATE users SET active_organization_id=NULL,organization_preference_revision=4 "
        "WHERE id=?",
        (USER,),
    )
    require(active_tenant(connection) is None, "absent selection loaded tenant data")
    choices = recovery_choices(connection)
    require(
        choices == [ORGANIZATION, OTHER_ORGANIZATION],
        f"absent-selection recovery choices drifted: {choices}",
    )
    run_selection_action_batch(
        connection, current_organization_id=None, expected_selection_revision=4
    )
    snapshot = selection_snapshot(connection)
    require(
        snapshot == selection_success_snapshot(5),
        f"absent-selection recovery did not commit exactly once: {snapshot}",
    )
    require(
        active_tenant(connection) == (OTHER_ORGANIZATION, "admin", 5),
        "recovered selection did not become the sole tenant authority",
    )
    return {
        "name": "absent_selection_recovered_from_authorized_choices",
        "choices": choices,
        "snapshot": snapshot,
    }


def stale_selection_recovery_case() -> dict[str, object]:
    connection = database()
    connection.execute(
        "UPDATE users SET organization_preference_revision=4 WHERE id=?", (USER,)
    )
    connection.execute(
        "UPDATE organization_members SET state='removed',revision=revision+1 "
        "WHERE organization_id=? AND user_id=?",
        (ORGANIZATION, USER),
    )
    require(active_tenant(connection) is None, "stale selection loaded tenant data")
    choices = recovery_choices(connection)
    require(
        choices == [OTHER_ORGANIZATION],
        f"stale-selection recovery exposed unauthorized choices: {choices}",
    )
    run_selection_action_batch(
        connection,
        current_organization_id=ORGANIZATION,
        expected_selection_revision=4,
    )
    snapshot = selection_snapshot(connection)
    require(
        snapshot == selection_success_snapshot(5),
        f"stale-selection recovery did not commit exactly once: {snapshot}",
    )
    return {
        "name": "stale_selection_recovered_from_authorized_choices",
        "choices": choices,
        "snapshot": snapshot,
    }


def selection_denied_case(
    name: str,
    fault: Callable[[sqlite3.Connection], None],
    expected_snapshot: dict[str, object],
) -> dict[str, object]:
    connection = database()
    current = active_tenant(connection)
    require(
        current == (ORGANIZATION, "owner", 3),
        f"{name}: initial selection authority drifted: {current}",
    )
    require(
        OTHER_ORGANIZATION in recovery_choices(connection),
        f"{name}: target membership was not admitted before the simulated race",
    )
    fault(connection)
    try:
        run_selection_action_batch(
            connection,
            current_organization_id=ORGANIZATION,
            expected_selection_revision=3,
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise ConformanceFailure(f"{name}: raced selector authority was accepted")
    snapshot = selection_snapshot(connection)
    require(
        snapshot == expected_snapshot,
        f"{name}: selector denial was not rollback-safe: {snapshot}",
    )
    return {"name": name, "rolled_back": True, "snapshot": snapshot}


def idempotent_selection_replay_case() -> dict[str, object]:
    connection = database()
    run_selection_action_batch(
        connection,
        current_organization_id=ORGANIZATION,
        expected_selection_revision=3,
    )
    stored = connection.execute(
        "SELECT operation_id,request_digest,state,response_json "
        "FROM authenticated_web_action_operations_v1 "
        "WHERE organization_id=? AND user_id=? AND action=? AND idempotency_key=?",
        (
            OTHER_ORGANIZATION,
            USER,
            SELECTION_ACTION,
            SELECTION_IDEMPOTENCY_KEY,
        ),
    ).fetchone()
    require(
        stored is not None
        and stored[0] == SELECTION_OPERATION
        and stored[1] == SELECTION_REQUEST_DIGEST
        and stored[2] == "complete"
        and stored[3],
        "completed selector receipt was unavailable for idempotent replay",
    )
    receipt = json.loads(str(stored[3]))
    require(
        receipt
        == {
            "schema_version": "frame.web-action-receipt.v1",
            "action": SELECTION_ACTION,
            "effect_state": "applied",
            "revision": 4,
            "invalidated": ["dashboard"],
        },
        f"completed selector receipt drifted: {receipt}",
    )
    issue_replay_grant(connection)
    run_selection_replay_consumption_batch(
        connection,
        grant_id=REPLAY_GRANT,
        assertion_operation_id=SELECTION_REPLAY_ASSERTION,
        expected_selection_revision=int(receipt["revision"]),
    )
    snapshot = selection_snapshot(connection)
    require(
        snapshot == selection_success_snapshot(4),
        f"selector replay duplicated or mutated the selection: {snapshot}",
    )
    stored_after = connection.execute(
        "SELECT operation_id,request_digest,state,response_json "
        "FROM authenticated_web_action_operations_v1 WHERE operation_id=?",
        (SELECTION_OPERATION,),
    ).fetchone()
    require(stored_after == stored, "selector replay replaced its durable receipt")
    return {
        "name": "completed_selection_replay_returns_one_receipt",
        "replayed": True,
        "snapshot": snapshot,
    }


def selection_cross_target_key_reuse_case() -> dict[str, object]:
    connection = database()
    run_selection_action_batch(
        connection,
        current_organization_id=ORGANIZATION,
        expected_selection_revision=3,
    )
    stored = connection.execute(
        "SELECT operation.organization_id,operation.request_digest,operation.state,"
        "operation.response_json,"
        "(SELECT COUNT(*) FROM authenticated_web_action_operations_v1 matches "
        "WHERE matches.user_id=? AND matches.action=? "
        "AND matches.idempotency_key=?) AS matching_count "
        "FROM authenticated_web_action_operations_v1 operation "
        "WHERE operation.user_id=? AND operation.action=? "
        "AND operation.idempotency_key=? "
        "ORDER BY operation.created_at_ms,operation.operation_id LIMIT 1",
        (
            USER,
            SELECTION_ACTION,
            SELECTION_IDEMPOTENCY_KEY,
            USER,
            SELECTION_ACTION,
            SELECTION_IDEMPOTENCY_KEY,
        ),
    ).fetchone()
    require(
        stored is not None
        and stored[0] == OTHER_ORGANIZATION
        and stored[1] == SELECTION_REQUEST_DIGEST
        and stored[2] == "complete"
        and stored[3]
        and stored[4] == 1,
        f"global selector idempotency lookup drifted: {stored}",
    )
    requested_target = ORGANIZATION
    require(
        requested_target != stored[0],
        "cross-target selector scenario did not change the requested target",
    )
    issue_replay_grant(connection)
    run_session_grant_consumption_batch(
        connection,
        grant_id=REPLAY_GRANT,
        assertion_operation_id=SELECTION_CROSS_TARGET_ASSERTION,
    )
    snapshot = selection_snapshot(connection)
    require(
        snapshot == selection_success_snapshot(4),
        f"cross-target key conflict created a second selector effect: {snapshot}",
    )
    require(
        connection.execute(
            "SELECT COUNT(*) FROM authenticated_web_action_operations_v1 "
            "WHERE organization_id=? AND user_id=? AND action=? AND idempotency_key=?",
            (
                requested_target,
                USER,
                SELECTION_ACTION,
                SELECTION_IDEMPOTENCY_KEY,
            ),
        ).fetchone()[0]
        == 0,
        "cross-target key reuse inserted a second selector operation",
    )
    return {
        "name": "selection_idempotency_key_reuse_across_targets_conflicts",
        "conflicted": True,
        "snapshot": snapshot,
    }


def selection_grant_is_one_use_case() -> dict[str, object]:
    connection = database()
    run_selection_action_batch(
        connection,
        current_organization_id=ORGANIZATION,
        expected_selection_revision=3,
    )
    before = selection_snapshot(connection)
    try:
        run_selection_replay_consumption_batch(
            connection,
            grant_id=GRANT,
            assertion_operation_id=SELECTION_REUSE_ASSERTION,
            expected_selection_revision=4,
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise ConformanceFailure("consumed selector grant was accepted a second time")
    after = selection_snapshot(connection)
    require(
        after == before == selection_success_snapshot(4),
        f"selector grant reuse changed durable state: before={before}, after={after}",
    )
    return {
        "name": "selection_mutation_grant_is_one_use",
        "second_use_denied": True,
        "snapshot": after,
    }


def selection_replay_membership_race_case() -> dict[str, object]:
    connection = database()
    run_selection_action_batch(
        connection,
        current_organization_id=ORGANIZATION,
        expected_selection_revision=3,
    )
    issue_replay_grant(connection)
    connection.execute(
        "UPDATE organization_members SET state='removed',revision=revision+1 "
        "WHERE organization_id=? AND user_id=?",
        (OTHER_ORGANIZATION, USER),
    )
    try:
        run_selection_replay_consumption_batch(
            connection,
            grant_id=REPLAY_GRANT,
            assertion_operation_id=SELECTION_REPLAY_ASSERTION,
            expected_selection_revision=4,
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise ConformanceFailure("selector replay accepted removed target membership")
    snapshot = selection_snapshot(connection)
    expected = selection_success_snapshot(4) | {"grants": 1}
    require(
        snapshot == expected,
        f"denied selector replay consumed its grant or changed receipt state: {snapshot}",
    )
    return {
        "name": "completed_selection_replay_target_membership_race",
        "rolled_back": True,
        "snapshot": snapshot,
    }


def selection_replay_selection_race_case() -> dict[str, object]:
    connection = database()
    run_selection_action_batch(
        connection,
        current_organization_id=ORGANIZATION,
        expected_selection_revision=3,
    )
    issue_replay_grant(connection)
    connection.execute(
        "UPDATE users SET active_organization_id=?,"
        "organization_preference_revision=organization_preference_revision+1 "
        "WHERE id=?",
        (ORGANIZATION, USER),
    )
    try:
        run_selection_replay_consumption_batch(
            connection,
            grant_id=REPLAY_GRANT,
            assertion_operation_id=SELECTION_REPLAY_ASSERTION,
            expected_selection_revision=4,
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise ConformanceFailure("selector replay accepted a concurrent selection change")
    snapshot = selection_snapshot(connection)
    expected = selection_success_snapshot(5) | {
        "active_organization_id": ORGANIZATION,
        "grants": 1,
    }
    require(
        snapshot == expected,
        f"stale selector replay consumed its grant or changed receipt state: {snapshot}",
    )
    return {
        "name": "completed_selection_replay_selection_revision_race",
        "rolled_back": True,
        "snapshot": snapshot,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path)
    args = parser.parse_args()
    cases = [
        success_case(),
        explicit_active_selection_case(),
        absent_selection_denied_case(),
        absent_selection_recovery_case(),
        stale_selection_recovery_case(),
        selection_denied_case(
            "selection_target_membership_removed_after_precheck",
            lambda connection: connection.execute(
                "UPDATE organization_members SET state='removed',revision=revision+1 "
                "WHERE organization_id=? AND user_id=?",
                (OTHER_ORGANIZATION, USER),
            ),
            selection_denied_snapshot(),
        ),
        selection_denied_case(
            "selection_target_role_became_viewer_after_precheck",
            lambda connection: connection.execute(
                "UPDATE organization_members SET role='viewer',revision=revision+1 "
                "WHERE organization_id=? AND user_id=?",
                (OTHER_ORGANIZATION, USER),
            ),
            selection_denied_snapshot(),
        ),
        selection_denied_case(
            "selection_revision_raced_after_precheck",
            lambda connection: connection.execute(
                "UPDATE users SET organization_preference_revision="
                "organization_preference_revision+1 WHERE id=?",
                (USER,),
            ),
            selection_denied_snapshot(selection_revision=4),
        ),
        selection_denied_case(
            "selection_value_raced_after_precheck",
            lambda connection: connection.execute(
                "UPDATE users SET active_organization_id=NULL,"
                "organization_preference_revision=organization_preference_revision+1 "
                "WHERE id=?",
                (USER,),
            ),
            selection_denied_snapshot(
                active_organization_id=None, selection_revision=4
            ),
        ),
        selection_denied_case(
            "selection_session_revoked_after_validation",
            lambda connection: connection.execute(
                "UPDATE auth_sessions_v2 SET state='revoked',revoked_at_ms=?,"
                "revocation_reason='operator',revision=revision+1 WHERE id=?",
                (NOW, SESSION),
            ),
            selection_denied_snapshot(),
        ),
        idempotent_selection_replay_case(),
        selection_cross_target_key_reuse_case(),
        selection_grant_is_one_use_case(),
        selection_replay_membership_race_case(),
        selection_replay_selection_race_case(),
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
        "active_selection_recovery": True,
        "active_selection_target_authorization": True,
        "active_selection_idempotent_replay": True,
        "active_selection_global_idempotency": True,
        "active_selection_one_use_grant": True,
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
