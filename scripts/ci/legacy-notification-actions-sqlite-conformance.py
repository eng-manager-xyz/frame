#!/usr/bin/env python3
"""Provider-free SQLite proof for legacy notification-action D1 semantics.

The suite applies the complete expand migration chain through 0038 and executes
the checked-in SQL used by the notification D1 adapter.  The adversarial cases
are intentional: a stale authority snapshot, spent browser proof, false
postcondition, conflicting idempotency key, or losing race must never leave a
partial product mutation or durable journal.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
import tempfile
import threading
from dataclasses import dataclass
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
MIGRATION_ROOT = ROOT / "apps/control-plane/migrations"
QUERY_ROOT = ROOT / "apps/control-plane/queries/legacy_notification_actions"
RUNTIME = ROOT / "apps/control-plane/src/legacy_notification_actions_runtime.rs"
APPLICATION = ROOT / "crates/application/src/legacy_notification_actions.rs"

NOW_MS = 1_700_000_000_000
MARK_ACTION = "legacy.notification.mark_as_read"
PREFERENCES_ACTION = "legacy.notification.update_preferences"


def uuid7_fixture(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


ACTOR = uuid7_fixture(1)
MEMBER = uuid7_fixture(2)
FOREIGN_ACTOR = uuid7_fixture(3)
SUSPENDED_ACTOR = uuid7_fixture(4)
ORGANIZATION = uuid7_fixture(10)
FOREIGN_ORGANIZATION = uuid7_fixture(11)

UNREAD_ONE = uuid7_fixture(100)
UNREAD_TWO = uuid7_fixture(101)
ALREADY_READ = uuid7_fixture(102)
FOREIGN_TENANT_NOTIFICATION = uuid7_fixture(103)
FOREIGN_RECIPIENT_NOTIFICATION = uuid7_fixture(104)

ORIGINAL_ACTOR_PREFERENCES: dict[str, Any] = {
    "appearance": {
        "theme": "dark",
        "nested": {"z": 1, "a": [3, 2, 1]},
    },
    "locale": "en-US",
    "notifications": {
        "pauseComments": False,
        "pauseReplies": False,
        "pauseViews": False,
        "pauseReactions": False,
        "pauseAnonViews": True,
    },
    "nullable": None,
}

SQL = {
    path.stem: path.read_text(encoding="utf-8").strip()
    for path in sorted(QUERY_ROOT.glob("*.sql"))
}


def digest(label: str) -> str:
    return hashlib.sha256(f"frame-notification-conformance:{label}".encode()).hexdigest()


def compact_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def sibling_digest(raw_preferences: str | None) -> str:
    if raw_preferences is None:
        value: Any = {}
    else:
        value = json.loads(raw_preferences)
        if value is None:
            value = {}
    if not isinstance(value, dict):
        raise ValueError("preferences authority is not a JSON object")
    siblings = {key: item for key, item in value.items() if key != "notifications"}
    return hashlib.sha256(compact_json(siblings).encode()).hexdigest()


def migration_paths(through: int = 38) -> list[Path]:
    return [
        path
        for path in sorted(MIGRATION_ROOT.glob("*.sql"))
        if int(path.name[:4]) <= through
    ]


def connect(path: Path | None = None) -> sqlite3.Connection:
    database = sqlite3.connect(
        ":memory:" if path is None else path,
        timeout=15,
        isolation_level=None,
    )
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    database.execute("PRAGMA busy_timeout = 15000")
    return database


def migrated_database(
    *, path: Path | None = None, through: int = 38
) -> sqlite3.Connection:
    database = connect(path)
    for migration in migration_paths(through):
        database.executescript(migration.read_text(encoding="utf-8"))
        violations = database.execute("PRAGMA foreign_key_check").fetchall()
        if violations:
            raise AssertionError(
                f"{migration.name} introduced foreign-key violations: {violations}"
            )
    return database


def seed_fixture(database: sqlite3.Connection) -> None:
    database.execute("BEGIN")
    try:
        users = (
            (ACTOR, "owner", "active", None, ORGANIZATION, ORIGINAL_ACTOR_PREFERENCES),
            (MEMBER, "member", "active", None, ORGANIZATION, {"locale": "fr-FR"}),
            (
                FOREIGN_ACTOR,
                "foreign",
                "active",
                None,
                FOREIGN_ORGANIZATION,
                {"appearance": {"theme": "light"}},
            ),
            (
                SUSPENDED_ACTOR,
                "suspended",
                "suspended",
                None,
                ORGANIZATION,
                {},
            ),
        )
        for user_id, name, status, deleted_at_ms, active_org, preferences in users:
            database.execute(
                """INSERT INTO users(
                     id, email, display_name, created_at_ms, updated_at_ms,
                     status, deleted_at_ms, active_organization_id,
                     organization_preference_revision, preferences_json
                   ) VALUES (?1, ?2, ?3, 1, 1, ?4, ?5, ?6, 7, ?7)""",
                (
                    user_id,
                    f"{name}@example.invalid",
                    name,
                    status,
                    deleted_at_ms,
                    active_org,
                    compact_json(preferences),
                ),
            )

        database.executemany(
            """INSERT INTO organizations(
                 id, owner_id, name, status, created_at_ms, updated_at_ms,
                 revision, authority_version
               ) VALUES (?1, ?2, ?3, 'active', 1, 1, 11, 13)""",
            (
                (ORGANIZATION, ACTOR, "primary"),
                (FOREIGN_ORGANIZATION, FOREIGN_ACTOR, "foreign"),
            ),
        )
        database.executemany(
            """INSERT INTO organization_members(
                 organization_id, user_id, role, state, created_at_ms,
                 updated_at_ms, revision, authority_version
               ) VALUES (?1, ?2, ?3, 'active', 1, 1, 5, 9)""",
            (
                (ORGANIZATION, ACTOR, "owner"),
                (ORGANIZATION, MEMBER, "member"),
                (FOREIGN_ORGANIZATION, FOREIGN_ACTOR, "owner"),
                (FOREIGN_ORGANIZATION, ACTOR, "viewer"),
            ),
        )
        database.executemany(
            """INSERT INTO notifications(
                 id, organization_id, recipient_user_id, type,
                 deduplication_key, data_json, created_at_ms, read_at_ms
               ) VALUES (?1, ?2, ?3, 'comment', ?4, '{}', ?5, ?6)""",
            (
                (UNREAD_ONE, ORGANIZATION, ACTOR, "unread-one", 10, None),
                (UNREAD_TWO, ORGANIZATION, ACTOR, "unread-two", 11, None),
                (ALREADY_READ, ORGANIZATION, ACTOR, "already-read", 12, NOW_MS - 50),
                (
                    FOREIGN_TENANT_NOTIFICATION,
                    FOREIGN_ORGANIZATION,
                    ACTOR,
                    "foreign-tenant",
                    13,
                    None,
                ),
                (
                    FOREIGN_RECIPIENT_NOTIFICATION,
                    ORGANIZATION,
                    MEMBER,
                    "foreign-recipient",
                    14,
                    None,
                ),
            ),
        )
        for user_id in (ACTOR, MEMBER, FOREIGN_ACTOR, SUSPENDED_ACTOR):
            database.execute(
                """INSERT INTO auth_identities_v2(
                     user_id, identity_revision, session_version,
                     created_at_ms, updated_at_ms, revision
                   ) VALUES (?1, 1, 3, 1, 1, 0)""",
                (user_id,),
            )
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


@dataclass(frozen=True)
class Grant:
    grant_id: str
    session_id: str
    user_id: str


def seed_grant(
    database: sqlite3.Connection, *, serial: int, user_id: str = ACTOR
) -> Grant:
    base = 10_000 + serial * 4
    session_id = uuid7_fixture(base)
    family_id = uuid7_fixture(base + 1)
    grant_id = uuid7_fixture(base + 2)
    token_digest = digest(f"token-{serial}-{user_id}")
    database.execute(
        """INSERT INTO auth_sessions_v2(
             id, family_id, user_id, client_kind, token_key_version,
             token_digest, csrf_key_version, csrf_digest, browser_origin,
             issued_at_ms, rotated_at_ms, idle_expires_at_ms,
             absolute_expires_at_ms, session_version, generation, state,
             revision, last_operation_id
           ) VALUES (
             ?1, ?2, ?3, 'browser', 7, ?4, 7, ?5,
             'https://frame.engmanager.xyz', ?6, ?6, ?7, ?8,
             3, 4, 'active', 0, ?9
           )""",
        (
            session_id,
            family_id,
            user_id,
            token_digest,
            digest(f"csrf-{serial}"),
            NOW_MS - 1_000,
            8_000_000_000_000_000,
            8_500_000_000_000_000,
            uuid7_fixture(base + 3),
        ),
    )
    database.execute(
        """INSERT INTO auth_session_mutation_grants_v2(
             id, session_id, user_id, generation, token_key_version,
             token_digest, created_at_ms, last_operation_id
           ) VALUES (?1, ?2, ?3, 4, 7, ?4, ?5, ?6)""",
        (
            grant_id,
            session_id,
            user_id,
            token_digest,
            NOW_MS - 500,
            uuid7_fixture(base + 3),
        ),
    )
    return Grant(grant_id, session_id, user_id)


def expect_integrity_error(
    database: sqlite3.Connection,
    sql: str,
    parameters: tuple[object, ...] = (),
    *,
    message: str | None = None,
) -> None:
    database.execute("SAVEPOINT expected_integrity_error")
    try:
        database.execute(sql, parameters)
    except sqlite3.IntegrityError as error:
        database.execute("ROLLBACK TO expected_integrity_error")
        database.execute("RELEASE expected_integrity_error")
        if message is not None and message not in str(error):
            raise AssertionError(
                f"expected trigger marker {message!r}, received {error!r}"
            ) from error
        return
    database.execute("ROLLBACK TO expected_integrity_error")
    database.execute("RELEASE expected_integrity_error")
    raise AssertionError("invalid notification journal mutation was accepted")


def test_full_migration_chain_and_expand_defaults() -> None:
    database = migrated_database(through=37)
    seed_fixture(database)
    before_preferences = database.execute(
        "SELECT preferences_json FROM users WHERE id=?1", (ACTOR,)
    ).fetchone()[0]
    before_notifications = database.execute(
        "SELECT id,read_at_ms FROM notifications ORDER BY id"
    ).fetchall()
    migration = MIGRATION_ROOT / "0038_legacy_notification_actions_expand.sql"
    database.executescript(migration.read_text(encoding="utf-8"))
    assert database.execute("PRAGMA foreign_key_check").fetchall() == []

    notification_columns = {
        row[1] for row in database.execute("PRAGMA table_info(notifications)")
    }
    user_columns = {row[1] for row in database.execute("PRAGMA table_info(users)")}
    assert {"revision", "last_operation_id"} <= notification_columns
    assert {
        "notification_preferences_revision",
        "notification_preferences_last_operation_id",
    } <= user_columns
    assert database.execute(
        "SELECT preferences_json FROM users WHERE id=?1", (ACTOR,)
    ).fetchone()[0] == before_preferences
    assert database.execute(
        "SELECT id,read_at_ms FROM notifications ORDER BY id"
    ).fetchall() == before_notifications
    assert database.execute(
        "SELECT COUNT(*) FROM notifications WHERE revision=0 AND last_operation_id IS NULL"
    ).fetchone()[0] == len(before_notifications)
    assert tuple(
        database.execute(
            "SELECT notification_preferences_revision,"
            "notification_preferences_last_operation_id FROM users WHERE id=?1",
            (ACTOR,),
        ).fetchone()
    ) == (0, None)

    tables = {
        row[0]
        for row in database.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        )
    }
    assert {
        "legacy_notification_action_operations_v1",
        "legacy_notification_action_receipts_v1",
        "legacy_notification_action_effects_v1",
        "legacy_notification_action_audit_events_v1",
        "legacy_notification_action_proof_consumptions_v1",
        "legacy_notification_action_assertions_v1",
    } <= tables
    triggers = {
        row[0]
        for row in database.execute(
            "SELECT name FROM sqlite_master WHERE type='trigger'"
        )
    }
    assert {
        "legacy_notification_action_operations_transition_v1",
        "legacy_notification_action_operations_delete_v1",
        "legacy_notification_action_receipts_update_v1",
        "legacy_notification_action_receipts_delete_v1",
        "legacy_notification_action_effects_update_v1",
        "legacy_notification_action_effects_delete_v1",
        "legacy_notification_action_audit_update_v1",
        "legacy_notification_action_audit_delete_v1",
        "legacy_notification_action_proofs_update_v1",
        "legacy_notification_action_proofs_delete_v1",
    } <= triggers
    database.close()


@dataclass(frozen=True)
class MarkPlan:
    operation_id: str
    audit_id: str
    grant: Grant
    selected_notification_id: str | None
    idempotency_key_digest: str
    request_digest: str
    authority: tuple[int, int, int, str, int, int]
    matching_count: int
    effect_json: str


@dataclass(frozen=True)
class PreferencesPlan:
    operation_id: str
    audit_id: str
    grant: Grant
    idempotency_key_digest: str
    request_digest: str
    original_preferences_json: str | None
    original_revision: int
    notifications: dict[str, bool]
    notifications_json: str
    merged_preferences_json: str
    preserved_digest: str
    effect_json: str


def mark_effect_json(organization_id: str = ORGANIZATION) -> str:
    assert organization_id == ORGANIZATION
    return (
        '{"invalidatesNotificationList":true,'
        '"invalidatesNotificationPreferences":false,'
        '"revalidationPath":"/dashboard",'
        '"schema":"frame.legacy-notification-effect.v1"}'
    )


def preferences_effect_json() -> str:
    return (
        '{"invalidatesNotificationList":false,'
        '"invalidatesNotificationPreferences":true,'
        '"revalidationPath":"/dashboard",'
        '"schema":"frame.legacy-notification-effect.v1"}'
    )


def one_row(
    database: sqlite3.Connection, sql: str, parameters: tuple[object, ...]
) -> sqlite3.Row:
    rows = database.execute(sql, parameters).fetchall()
    if len(rows) != 1:
        raise AssertionError(f"expected one bounded authority row, received {len(rows)}")
    return rows[0]


def mark_authority(
    database: sqlite3.Connection,
    *,
    actor_id: str = ACTOR,
    organization_id: str = ORGANIZATION,
) -> tuple[int, int, int, str, int, int]:
    row = one_row(
        database,
        SQL["mark_authority_snapshot"],
        (actor_id, organization_id),
    )
    return (
        row["selection_revision"],
        row["organization_revision"],
        row["organization_authority_version"],
        row["membership_role"],
        row["membership_revision"],
        row["membership_authority_version"],
    )


def insert_mark_authority_assertion(
    database: sqlite3.Connection,
    operation_id: str,
    authority: tuple[int, int, int, str, int, int],
    *,
    actor_id: str = ACTOR,
    organization_id: str = ORGANIZATION,
) -> None:
    database.execute(
        SQL["mark_authority_assert"],
        (operation_id, actor_id, organization_id, *authority),
    )


def mark_plan(
    database: sqlite3.Connection,
    *,
    serial: int,
    grant: Grant,
    selected_notification_id: str | None,
    idempotency_key_digest: str | None = None,
    request_digest: str | None = None,
) -> MarkPlan:
    matching_count = int(
        database.execute(
            SQL["mark_matching_count"],
            (ORGANIZATION, ACTOR, selected_notification_id),
        ).fetchone()["matching_count"]
    )
    return MarkPlan(
        operation_id=uuid7_fixture(20_000 + serial * 2),
        audit_id=uuid7_fixture(20_000 + serial * 2 + 1),
        grant=grant,
        selected_notification_id=selected_notification_id,
        idempotency_key_digest=idempotency_key_digest or digest(f"mark-key-{serial}"),
        request_digest=request_digest or digest(f"mark-request-{serial}"),
        authority=mark_authority(database),
        matching_count=matching_count,
        effect_json=mark_effect_json(),
    )


def preferences_plan(
    database: sqlite3.Connection,
    *,
    serial: int,
    grant: Grant,
    notifications: dict[str, bool],
    idempotency_key_digest: str | None = None,
    request_digest: str | None = None,
) -> PreferencesPlan:
    row = one_row(database, SQL["preferences_snapshot"], (ACTOR,))
    raw = row["preferences_json"]
    current: Any = {} if raw is None else json.loads(raw)
    if current is None:
        current = {}
    if not isinstance(current, dict):
        raise ValueError("preferences authority is not a JSON object")
    merged = dict(current)
    merged["notifications"] = notifications
    merged_json = compact_json(merged)
    before = sibling_digest(raw)
    after = sibling_digest(merged_json)
    assert before == after
    return PreferencesPlan(
        operation_id=uuid7_fixture(30_000 + serial * 2),
        audit_id=uuid7_fixture(30_000 + serial * 2 + 1),
        grant=grant,
        idempotency_key_digest=(
            idempotency_key_digest or digest(f"preferences-key-{serial}")
        ),
        request_digest=request_digest or digest(f"preferences-request-{serial}"),
        original_preferences_json=raw,
        original_revision=int(row["notification_preferences_revision"]),
        notifications=notifications,
        notifications_json=compact_json(notifications),
        merged_preferences_json=merged_json,
        preserved_digest=before,
        effect_json=preferences_effect_json(),
    )


def database_now(database: sqlite3.Connection) -> int:
    return int(database.execute(SQL["clock_now"]).fetchone()["now_ms"])


def insert_change_assertion(
    database: sqlite3.Connection,
    operation_id: str,
    assertion_kind: str,
    expected: int,
) -> None:
    database.execute(
        SQL["changes_assert"], (operation_id, assertion_kind, expected)
    )


def assert_browser_grant(
    database: sqlite3.Connection,
    operation_id: str,
    grant: Grant,
    now_ms: int,
) -> None:
    database.execute(
        SQL["browser_grant_assert"],
        (operation_id, grant.grant_id, grant.session_id, grant.user_id, now_ms),
    )


def consume_browser_grant(
    database: sqlite3.Connection,
    operation_id: str,
    grant: Grant,
) -> sqlite3.Row:
    rows = database.execute(
        SQL["browser_grant_delete_returning"],
        (grant.grant_id, grant.session_id, grant.user_id),
    ).fetchall()
    if len(rows) != 1:
        raise AssertionError("browser grant consumption did not return exactly one row")
    insert_change_assertion(database, operation_id, "grant_consumed", 1)
    consumed = rows[0]
    assert tuple(consumed) == (grant.grant_id, grant.session_id, grant.user_id)
    return consumed


def insert_audit(
    database: sqlite3.Connection,
    *,
    audit_id: str,
    operation_id: str,
    organization_id: str | None,
    action: str,
    request_digest: str,
    now_ms: int,
) -> None:
    database.execute(
        SQL["audit_insert"],
        (
            audit_id,
            operation_id,
            ACTOR,
            organization_id,
            action,
            digest("principal-subject"),
            digest(f"subject-{request_digest}"),
            now_ms,
        ),
    )
    insert_change_assertion(database, operation_id, "audit_inserted", 1)


def insert_proof(
    database: sqlite3.Connection,
    *,
    operation_id: str,
    grant: Grant,
    related_operation_id: str | None,
    tenant_kind: str,
    tenant_id: str,
    organization_id: str | None,
    action: str,
    request_digest: str,
    outcome: str,
    now_ms: int,
) -> None:
    database.execute(
        SQL["proof_insert"],
        (
            grant.grant_id,
            grant.session_id,
            ACTOR,
            related_operation_id,
            tenant_kind,
            tenant_id,
            organization_id,
            action,
            request_digest,
            outcome,
            now_ms,
        ),
    )
    insert_change_assertion(database, operation_id, "proof_journaled", 1)


def assert_durable_receipt(
    database: sqlite3.Connection,
    *,
    operation_id: str,
    tenant_kind: str,
    tenant_id: str,
    organization_id: str | None,
    action: str,
    request_digest: str,
    result_kind: str,
    selected_notification_id: str | None,
    matched_count: int | None,
    read_at_ms: int | None,
    notifications_json: str | None,
    preserved_before: str | None,
    preserved_after: str | None,
    matching_before: int,
    updated_rows: int,
    matching_after: int,
    out_of_scope_updated_rows: int,
    other_actor_rows_updated: int,
    effect_json: str,
    grant: Grant,
    proof_outcome: str,
) -> None:
    database.execute(
        SQL["durable_receipt_assert"],
        (
            operation_id,
            tenant_kind,
            tenant_id,
            organization_id,
            ACTOR,
            action,
            request_digest,
            result_kind,
            selected_notification_id,
            matched_count,
            read_at_ms,
            notifications_json,
            preserved_before,
            preserved_after,
            matching_before,
            updated_rows,
            matching_after,
            out_of_scope_updated_rows,
            other_actor_rows_updated,
            effect_json,
            grant.grant_id,
            grant.session_id,
            proof_outcome,
        ),
    )


def execute_mark(
    database: sqlite3.Connection,
    plan: MarkPlan,
    *,
    include_audit: bool = True,
    false_postcondition: bool = False,
) -> int:
    database.execute("BEGIN IMMEDIATE")
    try:
        now_ms = database_now(database)
        database.execute(
            SQL["operation_claim"],
            (
                plan.operation_id,
                "organization",
                ORGANIZATION,
                ORGANIZATION,
                ACTOR,
                MARK_ACTION,
                plan.idempotency_key_digest,
                plan.request_digest,
                now_ms,
            ),
        )
        assert_browser_grant(database, plan.operation_id, plan.grant, now_ms)
        insert_mark_authority_assertion(database, plan.operation_id, plan.authority)
        database.execute(
            SQL["mark_precondition_assert"],
            (
                plan.operation_id,
                ORGANIZATION,
                ACTOR,
                plan.selected_notification_id,
                plan.matching_count,
            ),
        )
        database.execute(
            SQL["mark_update"],
            (
                plan.operation_id,
                ORGANIZATION,
                ACTOR,
                plan.selected_notification_id,
                now_ms,
            ),
        )
        insert_change_assertion(
            database, plan.operation_id, "mark_updated", plan.matching_count
        )
        database.execute(
            SQL["mark_postcondition_assert"],
            (
                plan.operation_id,
                ORGANIZATION,
                ACTOR,
                plan.selected_notification_id,
                now_ms + int(false_postcondition),
                plan.matching_count,
            ),
        )
        database.execute(
            SQL["mark_out_of_scope_assert"],
            (
                plan.operation_id,
                ORGANIZATION,
                ACTOR,
                plan.selected_notification_id,
            ),
        )
        database.execute(
            SQL["receipt_insert"],
            (
                plan.operation_id,
                "marked_read",
                plan.selected_notification_id,
                plan.matching_count,
                now_ms,
                None,
                None,
                None,
                plan.matching_count,
                plan.matching_count,
                plan.matching_count,
                0,
                0,
                now_ms,
            ),
        )
        insert_change_assertion(database, plan.operation_id, "receipt_inserted", 1)
        database.execute(
            SQL["effect_insert"],
            (
                plan.operation_id,
                ACTOR,
                ORGANIZATION,
                MARK_ACTION,
                plan.effect_json,
                now_ms,
            ),
        )
        insert_change_assertion(database, plan.operation_id, "effect_inserted", 1)
        if include_audit:
            insert_audit(
                database,
                audit_id=plan.audit_id,
                operation_id=plan.operation_id,
                organization_id=ORGANIZATION,
                action=MARK_ACTION,
                request_digest=plan.request_digest,
                now_ms=now_ms,
            )
        consume_browser_grant(database, plan.operation_id, plan.grant)
        insert_proof(
            database,
            operation_id=plan.operation_id,
            grant=plan.grant,
            related_operation_id=plan.operation_id,
            tenant_kind="organization",
            tenant_id=ORGANIZATION,
            organization_id=ORGANIZATION,
            action=MARK_ACTION,
            request_digest=plan.request_digest,
            outcome="applied",
            now_ms=now_ms,
        )
        database.execute(SQL["operation_complete"], (plan.operation_id, now_ms))
        insert_change_assertion(database, plan.operation_id, "operation_complete", 1)
        assert_durable_receipt(
            database,
            operation_id=plan.operation_id,
            tenant_kind="organization",
            tenant_id=ORGANIZATION,
            organization_id=ORGANIZATION,
            action=MARK_ACTION,
            request_digest=plan.request_digest,
            result_kind="marked_read",
            selected_notification_id=plan.selected_notification_id,
            matched_count=plan.matching_count,
            read_at_ms=now_ms,
            notifications_json=None,
            preserved_before=None,
            preserved_after=None,
            matching_before=plan.matching_count,
            updated_rows=plan.matching_count,
            matching_after=plan.matching_count,
            out_of_scope_updated_rows=0,
            other_actor_rows_updated=0,
            effect_json=plan.effect_json,
            grant=plan.grant,
            proof_outcome="applied",
        )
        database.execute(SQL["assertion_cleanup"], (plan.operation_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")
    return now_ms


def execute_preferences(database: sqlite3.Connection, plan: PreferencesPlan) -> None:
    database.execute("BEGIN IMMEDIATE")
    try:
        now_ms = database_now(database)
        database.execute(
            SQL["operation_claim"],
            (
                plan.operation_id,
                "actor",
                ACTOR,
                None,
                ACTOR,
                PREFERENCES_ACTION,
                plan.idempotency_key_digest,
                plan.request_digest,
                now_ms,
            ),
        )
        assert_browser_grant(database, plan.operation_id, plan.grant, now_ms)
        database.execute(
            SQL["preferences_authority_assert"],
            (
                plan.operation_id,
                ACTOR,
                plan.original_revision,
                plan.original_preferences_json,
            ),
        )
        database.execute(
            SQL["preferences_update"],
            (
                plan.operation_id,
                ACTOR,
                plan.original_revision,
                plan.original_preferences_json,
                plan.merged_preferences_json,
            ),
        )
        insert_change_assertion(database, plan.operation_id, "preferences_updated", 1)
        database.execute(
            SQL["preferences_postcondition_assert"],
            (
                plan.operation_id,
                ACTOR,
                plan.original_revision,
                plan.merged_preferences_json,
                plan.notifications_json,
            ),
        )
        database.execute(
            SQL["preferences_other_actor_assert"],
            (plan.operation_id, ACTOR),
        )
        database.execute(
            SQL["receipt_insert"],
            (
                plan.operation_id,
                "preferences_updated",
                None,
                None,
                None,
                plan.notifications_json,
                plan.preserved_digest,
                plan.preserved_digest,
                1,
                1,
                1,
                0,
                0,
                now_ms,
            ),
        )
        insert_change_assertion(database, plan.operation_id, "receipt_inserted", 1)
        database.execute(
            SQL["effect_insert"],
            (
                plan.operation_id,
                ACTOR,
                None,
                PREFERENCES_ACTION,
                plan.effect_json,
                now_ms,
            ),
        )
        insert_change_assertion(database, plan.operation_id, "effect_inserted", 1)
        insert_audit(
            database,
            audit_id=plan.audit_id,
            operation_id=plan.operation_id,
            organization_id=None,
            action=PREFERENCES_ACTION,
            request_digest=plan.request_digest,
            now_ms=now_ms,
        )
        consume_browser_grant(database, plan.operation_id, plan.grant)
        insert_proof(
            database,
            operation_id=plan.operation_id,
            grant=plan.grant,
            related_operation_id=plan.operation_id,
            tenant_kind="actor",
            tenant_id=ACTOR,
            organization_id=None,
            action=PREFERENCES_ACTION,
            request_digest=plan.request_digest,
            outcome="applied",
            now_ms=now_ms,
        )
        database.execute(SQL["operation_complete"], (plan.operation_id, now_ms))
        insert_change_assertion(database, plan.operation_id, "operation_complete", 1)
        assert_durable_receipt(
            database,
            operation_id=plan.operation_id,
            tenant_kind="actor",
            tenant_id=ACTOR,
            organization_id=None,
            action=PREFERENCES_ACTION,
            request_digest=plan.request_digest,
            result_kind="preferences_updated",
            selected_notification_id=None,
            matched_count=None,
            read_at_ms=None,
            notifications_json=plan.notifications_json,
            preserved_before=plan.preserved_digest,
            preserved_after=plan.preserved_digest,
            matching_before=1,
            updated_rows=1,
            matching_after=1,
            out_of_scope_updated_rows=0,
            other_actor_rows_updated=0,
            effect_json=plan.effect_json,
            grant=plan.grant,
            proof_outcome="applied",
        )
        database.execute(SQL["assertion_cleanup"], (plan.operation_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


def operation_by_key(
    database: sqlite3.Connection,
    *,
    tenant_kind: str,
    tenant_id: str,
    action: str,
    idempotency_key_digest: str,
) -> sqlite3.Row | None:
    rows = database.execute(
        SQL["operation_by_key"],
        (tenant_kind, tenant_id, ACTOR, action, idempotency_key_digest),
    ).fetchall()
    if len(rows) > 1:
        raise AssertionError("idempotency lookup returned ambiguous operations")
    return rows[0] if rows else None


def insert_current_authority_assertion(
    database: sqlite3.Connection,
    *,
    operation_id: str,
    action: str,
) -> None:
    if action == MARK_ACTION:
        insert_mark_authority_assertion(
            database, operation_id, mark_authority(database)
        )
        return
    row = one_row(database, SQL["preferences_snapshot"], (ACTOR,))
    database.execute(
        SQL["preferences_authority_assert"],
        (
            operation_id,
            ACTOR,
            row["notification_preferences_revision"],
            row["preferences_json"],
        ),
    )


def consume_retry(
    database: sqlite3.Connection,
    *,
    winner: sqlite3.Row,
    grant: Grant,
    tenant_kind: str,
    tenant_id: str,
    organization_id: str | None,
    action: str,
    request_digest: str,
    outcome: str,
) -> None:
    assert outcome in ("replay", "conflict", "in_flight", "rejected")
    operation_id = str(winner["operation_id"])
    database.execute("BEGIN IMMEDIATE")
    try:
        now_ms = database_now(database)
        assert_browser_grant(database, operation_id, grant, now_ms)
        insert_current_authority_assertion(
            database, operation_id=operation_id, action=action
        )
        consume_browser_grant(database, operation_id, grant)
        insert_proof(
            database,
            operation_id=operation_id,
            grant=grant,
            related_operation_id=operation_id,
            tenant_kind=tenant_kind,
            tenant_id=tenant_id,
            organization_id=organization_id,
            action=action,
            request_digest=request_digest,
            outcome=outcome,
            now_ms=now_ms,
        )
        if outcome == "replay":
            assert winner["state"] == "complete"
            assert winner["request_digest"] == request_digest
            assert_durable_receipt(
                database,
                operation_id=operation_id,
                tenant_kind=tenant_kind,
                tenant_id=tenant_id,
                organization_id=organization_id,
                action=action,
                request_digest=request_digest,
                result_kind=winner["result_kind"],
                selected_notification_id=winner["selected_notification_id"],
                matched_count=winner["matched_count"],
                read_at_ms=winner["read_at_ms"],
                notifications_json=winner["notifications_json"],
                preserved_before=winner["preserved_before_sha256"],
                preserved_after=winner["preserved_after_sha256"],
                matching_before=winner["matching_before"],
                updated_rows=winner["updated_rows"],
                matching_after=winner["matching_after"],
                out_of_scope_updated_rows=winner["out_of_scope_updated_rows"],
                other_actor_rows_updated=winner["other_actor_rows_updated"],
                effect_json=winner["effect_json"],
                grant=grant,
                proof_outcome="replay",
            )
        database.execute(SQL["assertion_cleanup"], (operation_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


def test_selected_and_bulk_mark_are_exactly_scoped() -> None:
    database = migrated_database()
    seed_fixture(database)
    grant = seed_grant(database, serial=1)
    plan = mark_plan(
        database,
        serial=1,
        grant=grant,
        selected_notification_id=UNREAD_ONE,
    )
    assert plan.matching_count == 1
    read_at_ms = execute_mark(database, plan)
    assert read_at_ms > NOW_MS
    assert tuple(
        database.execute(
            "SELECT read_at_ms,revision,last_operation_id FROM notifications WHERE id=?1",
            (UNREAD_ONE,),
        ).fetchone()
    ) == (read_at_ms, 1, plan.operation_id)
    assert tuple(
        database.execute(
            "SELECT read_at_ms,revision,last_operation_id FROM notifications WHERE id=?1",
            (UNREAD_TWO,),
        ).fetchone()
    ) == (None, 0, None)

    # A selected absent, cross-tenant, or other-recipient notification has the
    # same successful zero-row shape and never discloses which mismatch won.
    for serial, selected in enumerate(
        (
            uuid7_fixture(999_999),
            FOREIGN_TENANT_NOTIFICATION,
            FOREIGN_RECIPIENT_NOTIFICATION,
        ),
        start=2,
    ):
        zero_grant = seed_grant(database, serial=serial)
        zero = mark_plan(
            database,
            serial=serial,
            grant=zero_grant,
            selected_notification_id=selected,
        )
        assert zero.matching_count == 0
        execute_mark(database, zero)
        receipt = database.execute(
            """SELECT result_kind,selected_notification_id,matched_count,
                      matching_before,updated_rows,matching_after,
                      out_of_scope_updated_rows
               FROM legacy_notification_action_receipts_v1
               WHERE operation_id=?1""",
            (zero.operation_id,),
        ).fetchone()
        assert tuple(receipt) == ("marked_read", selected, 0, 0, 0, 0, 0)

    assert tuple(
        database.execute(
            "SELECT read_at_ms,revision,last_operation_id FROM notifications WHERE id=?1",
            (FOREIGN_TENANT_NOTIFICATION,),
        ).fetchone()
    ) == (None, 0, None)
    assert tuple(
        database.execute(
            "SELECT read_at_ms,revision,last_operation_id FROM notifications WHERE id=?1",
            (FOREIGN_RECIPIENT_NOTIFICATION,),
        ).fetchone()
    ) == (None, 0, None)
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_notification_action_assertions_v1"
    ).fetchone()[0] == 0
    database.close()

    database = migrated_database()
    seed_fixture(database)
    before_out_of_scope = {
        row["id"]: tuple(row)
        for row in database.execute(
            """SELECT id,read_at_ms,revision,last_operation_id
               FROM notifications
               WHERE id IN (?1,?2)
               ORDER BY id""",
            (FOREIGN_TENANT_NOTIFICATION, FOREIGN_RECIPIENT_NOTIFICATION),
        )
    }
    grant = seed_grant(database, serial=10)
    bulk = mark_plan(
        database,
        serial=10,
        grant=grant,
        selected_notification_id=None,
    )
    assert bulk.matching_count == 3
    bulk_read_at = execute_mark(database, bulk)
    rows = database.execute(
        """SELECT id,read_at_ms,revision,last_operation_id
           FROM notifications
           WHERE organization_id=?1 AND recipient_user_id=?2
           ORDER BY id""",
        (ORGANIZATION, ACTOR),
    ).fetchall()
    assert len(rows) == 3
    assert all(tuple(row)[1:] == (bulk_read_at, 1, bulk.operation_id) for row in rows)
    after_out_of_scope = {
        row["id"]: tuple(row)
        for row in database.execute(
            """SELECT id,read_at_ms,revision,last_operation_id
               FROM notifications
               WHERE id IN (?1,?2)
               ORDER BY id""",
            (FOREIGN_TENANT_NOTIFICATION, FOREIGN_RECIPIENT_NOTIFICATION),
        )
    }
    assert after_out_of_scope == before_out_of_scope
    winner = operation_by_key(
        database,
        tenant_kind="organization",
        tenant_id=ORGANIZATION,
        action=MARK_ACTION,
        idempotency_key_digest=bulk.idempotency_key_digest,
    )
    assert winner is not None
    assert winner["state"] == "complete"
    assert winner["matched_count"] == 3
    assert winner["read_at_ms"] == bulk_read_at
    assert winner["matching_before"] == winner["updated_rows"] == 3
    assert winner["matching_after"] == 3
    assert winner["out_of_scope_updated_rows"] == 0
    assert winner["audit_count"] == winner["proof_count"] == 1
    database.close()


def test_preferences_merge_preserves_siblings_and_optional_absence() -> None:
    database = migrated_database()
    seed_fixture(database)
    member_before = database.execute(
        "SELECT preferences_json FROM users WHERE id=?1", (MEMBER,)
    ).fetchone()[0]
    notifications = {
        "pauseComments": True,
        "pauseReplies": False,
        "pauseViews": True,
        "pauseReactions": False,
    }
    grant = seed_grant(database, serial=20)
    plan = preferences_plan(
        database, serial=20, grant=grant, notifications=notifications
    )
    execute_preferences(database, plan)

    user = database.execute(
        """SELECT preferences_json,notification_preferences_revision,
                  notification_preferences_last_operation_id
           FROM users WHERE id=?1""",
        (ACTOR,),
    ).fetchone()
    stored = json.loads(user["preferences_json"])
    expected_siblings = dict(ORIGINAL_ACTOR_PREFERENCES)
    expected_siblings.pop("notifications")
    stored_siblings = dict(stored)
    stored_notifications = stored_siblings.pop("notifications")
    assert stored_siblings == expected_siblings
    assert stored_notifications == notifications
    assert "pauseAnonViews" not in stored_notifications
    assert tuple(user)[1:] == (1, plan.operation_id)
    assert database.execute(
        "SELECT preferences_json FROM users WHERE id=?1", (MEMBER,)
    ).fetchone()[0] == member_before

    receipt = database.execute(
        """SELECT notifications_json,preserved_before_sha256,
                  preserved_after_sha256,matching_before,updated_rows,
                  matching_after,other_actor_rows_updated
           FROM legacy_notification_action_receipts_v1
           WHERE operation_id=?1""",
        (plan.operation_id,),
    ).fetchone()
    assert json.loads(receipt["notifications_json"]) == notifications
    assert receipt["preserved_before_sha256"] == plan.preserved_digest
    assert receipt["preserved_after_sha256"] == plan.preserved_digest
    assert tuple(receipt)[3:] == (1, 1, 1, 0)
    assert "appearance" not in receipt["notifications_json"]

    # Actor-global preference replay is independent of active organization.
    database.execute(
        """UPDATE users
           SET active_organization_id=?1,
               organization_preference_revision=organization_preference_revision+1
           WHERE id=?2""",
        (FOREIGN_ORGANIZATION, ACTOR),
    )
    winner = operation_by_key(
        database,
        tenant_kind="actor",
        tenant_id=ACTOR,
        action=PREFERENCES_ACTION,
        idempotency_key_digest=plan.idempotency_key_digest,
    )
    assert winner is not None
    replay_grant = seed_grant(database, serial=21)
    consume_retry(
        database,
        winner=winner,
        grant=replay_grant,
        tenant_kind="actor",
        tenant_id=ACTOR,
        organization_id=None,
        action=PREFERENCES_ACTION,
        request_digest=plan.request_digest,
        outcome="replay",
    )
    after_replay = database.execute(
        """SELECT preferences_json,notification_preferences_revision,
                  notification_preferences_last_operation_id
           FROM users WHERE id=?1""",
        (ACTOR,),
    ).fetchone()
    assert tuple(after_replay) == tuple(user)
    winner = operation_by_key(
        database,
        tenant_kind="actor",
        tenant_id=ACTOR,
        action=PREFERENCES_ACTION,
        idempotency_key_digest=plan.idempotency_key_digest,
    )
    assert winner["proof_count"] == 2

    explicit_false = dict(notifications)
    explicit_false["pauseAnonViews"] = False
    next_grant = seed_grant(database, serial=22)
    next_plan = preferences_plan(
        database,
        serial=22,
        grant=next_grant,
        notifications=explicit_false,
    )
    execute_preferences(database, next_plan)
    latest = json.loads(
        database.execute(
            "SELECT preferences_json FROM users WHERE id=?1", (ACTOR,)
        ).fetchone()[0]
    )
    assert latest["notifications"]["pauseAnonViews"] is False
    latest_siblings = dict(latest)
    latest_siblings.pop("notifications")
    assert latest_siblings == expected_siblings
    database.close()


def test_replay_conflict_in_flight_and_one_use_proof() -> None:
    database = migrated_database()
    seed_fixture(database)
    raw_key = "never-persist-this-notification-key"
    key_digest = digest(raw_key)
    request_digest = digest("stable-mark-request")
    first_grant = seed_grant(database, serial=30)
    plan = mark_plan(
        database,
        serial=30,
        grant=first_grant,
        selected_notification_id=UNREAD_ONE,
        idempotency_key_digest=key_digest,
        request_digest=request_digest,
    )
    first_read_at = execute_mark(database, plan)
    winner = operation_by_key(
        database,
        tenant_kind="organization",
        tenant_id=ORGANIZATION,
        action=MARK_ACTION,
        idempotency_key_digest=key_digest,
    )
    assert winner is not None
    assert winner["request_digest"] == request_digest
    assert winner["state"] == "complete"
    assert database.execute(
        SQL["operation_by_key"],
        ("organization", ORGANIZATION, MEMBER, MARK_ACTION, key_digest),
    ).fetchone() is None
    assert database.execute(
        SQL["operation_by_key"],
        (
            "organization",
            FOREIGN_ORGANIZATION,
            ACTOR,
            MARK_ACTION,
            key_digest,
        ),
    ).fetchone() is None
    assert database.execute(
        SQL["operation_by_key"],
        ("actor", ACTOR, ACTOR, PREFERENCES_ACTION, key_digest),
    ).fetchone() is None

    replay_grant = seed_grant(database, serial=31)
    consume_retry(
        database,
        winner=winner,
        grant=replay_grant,
        tenant_kind="organization",
        tenant_id=ORGANIZATION,
        organization_id=ORGANIZATION,
        action=MARK_ACTION,
        request_digest=request_digest,
        outcome="replay",
    )
    assert tuple(
        database.execute(
            "SELECT read_at_ms,revision,last_operation_id FROM notifications WHERE id=?1",
            (UNREAD_ONE,),
        ).fetchone()
    ) == (first_read_at, 1, plan.operation_id)

    conflicting_request = digest("different-mark-request")
    assert conflicting_request != winner["request_digest"]
    conflict_grant = seed_grant(database, serial=32)
    consume_retry(
        database,
        winner=winner,
        grant=conflict_grant,
        tenant_kind="organization",
        tenant_id=ORGANIZATION,
        organization_id=ORGANIZATION,
        action=MARK_ACTION,
        request_digest=conflicting_request,
        outcome="conflict",
    )
    assert tuple(
        database.execute(
            "SELECT read_at_ms,revision,last_operation_id FROM notifications WHERE id=?1",
            (UNREAD_ONE,),
        ).fetchone()
    ) == (first_read_at, 1, plan.operation_id)
    assert database.execute(
        """SELECT COUNT(*)
           FROM legacy_notification_action_proof_consumptions_v1
           WHERE mutation_grant_id=?1 AND related_operation_id=?2
             AND request_digest=?3 AND outcome='conflict'""",
        (conflict_grant.grant_id, plan.operation_id, conflicting_request),
    ).fetchone()[0] == 1

    # Reusing a consumed proof cannot create another proof row or touch data.
    proof_count = database.execute(
        "SELECT COUNT(*) FROM legacy_notification_action_proof_consumptions_v1"
    ).fetchone()[0]
    try:
        consume_retry(
            database,
            winner=winner,
            grant=first_grant,
            tenant_kind="organization",
            tenant_id=ORGANIZATION,
            organization_id=ORGANIZATION,
            action=MARK_ACTION,
            request_digest=request_digest,
            outcome="replay",
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("spent browser proof was accepted a second time")
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_notification_action_proof_consumptions_v1"
    ).fetchone()[0] == proof_count
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_notification_action_operations_v1"
    ).fetchone()[0] == 1
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_notification_action_receipts_v1"
    ).fetchone()[0] == 1
    winner = operation_by_key(
        database,
        tenant_kind="organization",
        tenant_id=ORGANIZATION,
        action=MARK_ACTION,
        idempotency_key_digest=key_digest,
    )
    assert winner["proof_count"] == 2
    assert raw_key not in "\n".join(database.iterdump())
    database.close()

    database = migrated_database()
    seed_fixture(database)
    in_flight_key = digest("in-flight-key")
    in_flight_request = digest("in-flight-request")
    operation_id = uuid7_fixture(88_000)
    database.execute(
        SQL["operation_claim"],
        (
            operation_id,
            "organization",
            ORGANIZATION,
            ORGANIZATION,
            ACTOR,
            MARK_ACTION,
            in_flight_key,
            in_flight_request,
            NOW_MS,
        ),
    )
    claimed = operation_by_key(
        database,
        tenant_kind="organization",
        tenant_id=ORGANIZATION,
        action=MARK_ACTION,
        idempotency_key_digest=in_flight_key,
    )
    assert claimed is not None
    assert claimed["state"] == "claimed"
    assert claimed["result_kind"] is None
    contender_grant = seed_grant(database, serial=33)
    consume_retry(
        database,
        winner=claimed,
        grant=contender_grant,
        tenant_kind="organization",
        tenant_id=ORGANIZATION,
        organization_id=ORGANIZATION,
        action=MARK_ACTION,
        request_digest=in_flight_request,
        outcome="in_flight",
    )
    assert database.execute(
        """SELECT COUNT(*)
           FROM legacy_notification_action_proof_consumptions_v1
           WHERE mutation_grant_id=?1 AND related_operation_id=?2
             AND outcome='in_flight'""",
        (contender_grant.grant_id, operation_id),
    ).fetchone()[0] == 1
    assert database.execute(
        "SELECT state FROM legacy_notification_action_operations_v1 WHERE operation_id=?1",
        (operation_id,),
    ).fetchone()[0] == "claimed"
    for table in (
        "legacy_notification_action_receipts_v1",
        "legacy_notification_action_effects_v1",
        "legacy_notification_action_audit_events_v1",
    ):
        assert database.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (contender_grant.grant_id,),
    ).fetchone()[0] == 0
    database.close()


def test_preferences_stale_snapshot_and_malformed_authority_fail_closed() -> None:
    notifications = {
        "pauseComments": True,
        "pauseReplies": False,
        "pauseViews": True,
        "pauseReactions": False,
    }
    # A sibling write after discovery invalidates the exact JSON/revision CAS.
    database = migrated_database()
    seed_fixture(database)
    stale_grant = seed_grant(database, serial=23)
    stale = preferences_plan(
        database,
        serial=23,
        grant=stale_grant,
        notifications=notifications,
    )
    concurrent = dict(ORIGINAL_ACTOR_PREFERENCES)
    concurrent["appearance"] = {"theme": "contrast", "fontScale": 1.25}
    database.execute(
        """UPDATE users
           SET preferences_json=?1,
               notification_preferences_revision=notification_preferences_revision+1
           WHERE id=?2""",
        (compact_json(concurrent), ACTOR),
    )
    try:
        execute_preferences(database, stale)
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("stale sibling preference snapshot committed")
    assert json.loads(
        database.execute(
            "SELECT preferences_json FROM users WHERE id=?1", (ACTOR,)
        ).fetchone()[0]
    ) == concurrent
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_notification_action_operations_v1"
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (stale_grant.grant_id,),
    ).fetchone()[0] == 1

    database.execute(
        "UPDATE users SET preferences_json='null' WHERE id=?1", (ACTOR,)
    )
    null_grant = seed_grant(database, serial=24)
    null_plan = preferences_plan(
        database,
        serial=24,
        grant=null_grant,
        notifications=notifications,
    )
    assert null_plan.preserved_digest == hashlib.sha256(b"{}").hexdigest()
    execute_preferences(database, null_plan)
    null_merged = json.loads(
        database.execute(
            "SELECT preferences_json FROM users WHERE id=?1", (ACTOR,)
        ).fetchone()[0]
    )
    assert null_merged == {"notifications": notifications}

    database.execute(
        "UPDATE users SET preferences_json='[]' WHERE id=?1", (ACTOR,)
    )
    malformed_grant = seed_grant(database, serial=25)
    try:
        preferences_plan(
            database,
            serial=25,
            grant=malformed_grant,
            notifications=notifications,
        )
    except ValueError:
        pass
    else:
        raise AssertionError("non-object preferences authority did not fail closed")
    database.close()


def notification_product_snapshot(database: sqlite3.Connection) -> list[tuple[Any, ...]]:
    return [
        tuple(row)
        for row in database.execute(
            """SELECT id,read_at_ms,revision,last_operation_id
               FROM notifications ORDER BY id"""
        )
    ]


def assert_no_notification_journal(database: sqlite3.Connection) -> None:
    for table in (
        "legacy_notification_action_operations_v1",
        "legacy_notification_action_receipts_v1",
        "legacy_notification_action_effects_v1",
        "legacy_notification_action_audit_events_v1",
        "legacy_notification_action_proof_consumptions_v1",
        "legacy_notification_action_assertions_v1",
    ):
        assert database.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0] == 0


def prove_mark_failure_rolls_back(
    *,
    serial: int,
    mutate_before_execute: Any | None = None,
    include_audit: bool = True,
    false_postcondition: bool = False,
    expected_grant_rows: int = 1,
) -> None:
    database = migrated_database()
    seed_fixture(database)
    grant = seed_grant(database, serial=serial)
    plan = mark_plan(
        database,
        serial=serial,
        grant=grant,
        selected_notification_id=UNREAD_ONE,
    )
    before = notification_product_snapshot(database)
    if mutate_before_execute is not None:
        mutate_before_execute(database, grant)
    try:
        execute_mark(
            database,
            plan,
            include_audit=include_audit,
            false_postcondition=false_postcondition,
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("invalid notification batch committed")
    assert notification_product_snapshot(database) == before
    assert_no_notification_journal(database)
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (grant.grant_id,),
    ).fetchone()[0] == expected_grant_rows
    database.close()


def test_failure_paths_are_atomic_and_fail_closed() -> None:
    # Receipt, effect, audit, proof, operation completion, and product write
    # form one postcondition. Missing audit evidence unwinds every one of them.
    prove_mark_failure_rolls_back(serial=40, include_audit=False)

    # A false exact-timestamp postcondition cannot preserve the product write.
    prove_mark_failure_rolls_back(serial=41, false_postcondition=True)

    def rotate_selection(database: sqlite3.Connection, _grant: Grant) -> None:
        database.execute(
            """UPDATE users
               SET organization_preference_revision=organization_preference_revision+1
               WHERE id=?1""",
            (ACTOR,),
        )

    prove_mark_failure_rolls_back(
        serial=42, mutate_before_execute=rotate_selection
    )

    def change_active_tenant(database: sqlite3.Connection, _grant: Grant) -> None:
        database.execute(
            """UPDATE users
               SET active_organization_id=?1,
                   organization_preference_revision=organization_preference_revision+1
               WHERE id=?2""",
            (FOREIGN_ORGANIZATION, ACTOR),
        )

    prove_mark_failure_rolls_back(
        serial=43, mutate_before_execute=change_active_tenant
    )

    def suspend_actor(database: sqlite3.Connection, _grant: Grant) -> None:
        database.execute(
            "UPDATE users SET status='suspended' WHERE id=?1", (ACTOR,)
        )

    prove_mark_failure_rolls_back(serial=44, mutate_before_execute=suspend_actor)

    def spend_grant(database: sqlite3.Connection, grant: Grant) -> None:
        database.execute(
            "DELETE FROM auth_session_mutation_grants_v2 WHERE id=?1",
            (grant.grant_id,),
        )

    prove_mark_failure_rolls_back(
        serial=45,
        mutate_before_execute=spend_grant,
        expected_grant_rows=0,
    )


def test_concurrent_same_key_has_one_mutation_winner() -> None:
    with tempfile.TemporaryDirectory(prefix="frame-notification-race-") as directory:
        path = Path(directory) / "notification.sqlite3"
        database = migrated_database(path=path)
        seed_fixture(database)
        database.execute("PRAGMA journal_mode = WAL")
        grant_a = seed_grant(database, serial=50)
        grant_b = seed_grant(database, serial=51)
        race_key = digest("race-key")
        race_request = digest("race-request")
        plan_a = mark_plan(
            database,
            serial=50,
            grant=grant_a,
            selected_notification_id=UNREAD_ONE,
            idempotency_key_digest=race_key,
            request_digest=race_request,
        )
        plan_b = mark_plan(
            database,
            serial=51,
            grant=grant_b,
            selected_notification_id=UNREAD_ONE,
            idempotency_key_digest=race_key,
            request_digest=race_request,
        )
        database.close()
        barrier = threading.Barrier(2)
        outcomes: list[str] = []
        failures: list[str] = []
        outcome_lock = threading.Lock()

        def contender(plan: MarkPlan) -> None:
            candidate = connect(path)
            try:
                barrier.wait(timeout=5)
                try:
                    execute_mark(candidate, plan)
                    outcome = "applied"
                except sqlite3.IntegrityError:
                    winner = operation_by_key(
                        candidate,
                        tenant_kind="organization",
                        tenant_id=ORGANIZATION,
                        action=MARK_ACTION,
                        idempotency_key_digest=race_key,
                    )
                    if (
                        winner is None
                        or winner["request_digest"] != race_request
                        or winner["state"] != "complete"
                    ):
                        raise AssertionError("race loser could not load exact winner")
                    consume_retry(
                        candidate,
                        winner=winner,
                        grant=plan.grant,
                        tenant_kind="organization",
                        tenant_id=ORGANIZATION,
                        organization_id=ORGANIZATION,
                        action=MARK_ACTION,
                        request_digest=race_request,
                        outcome="replay",
                    )
                    outcome = "replay"
                with outcome_lock:
                    outcomes.append(outcome)
            except Exception as error:  # reported after both threads join
                with outcome_lock:
                    failures.append(f"{type(error).__name__}: {error}")
            finally:
                candidate.close()

        threads = [
            threading.Thread(target=contender, args=(plan_a,)),
            threading.Thread(target=contender, args=(plan_b,)),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=20)
        if any(thread.is_alive() for thread in threads):
            raise AssertionError("notification idempotency race did not terminate")
        if failures:
            raise AssertionError(f"notification idempotency race failures: {failures}")
        assert sorted(outcomes) == ["applied", "replay"]

        database = connect(path)
        for table in (
            "legacy_notification_action_operations_v1",
            "legacy_notification_action_receipts_v1",
            "legacy_notification_action_effects_v1",
            "legacy_notification_action_audit_events_v1",
        ):
            assert database.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0] == 1
        assert database.execute(
            "SELECT COUNT(*) FROM legacy_notification_action_proof_consumptions_v1"
        ).fetchone()[0] == 2
        assert database.execute(
            "SELECT COUNT(*) FROM auth_session_mutation_grants_v2"
        ).fetchone()[0] == 0
        row = database.execute(
            "SELECT read_at_ms,revision,last_operation_id FROM notifications WHERE id=?1",
            (UNREAD_ONE,),
        ).fetchone()
        assert row["read_at_ms"] is not None
        assert row["revision"] == 1
        assert row["last_operation_id"] in (plan_a.operation_id, plan_b.operation_id)
        assert database.execute(
            "SELECT COUNT(*) FROM legacy_notification_action_assertions_v1"
        ).fetchone()[0] == 0
        database.close()


def test_typed_journal_constraints_and_immutability() -> None:
    database = migrated_database()
    seed_fixture(database)
    grant = seed_grant(database, serial=60)
    plan = mark_plan(
        database,
        serial=60,
        grant=grant,
        selected_notification_id=UNREAD_ONE,
    )
    execute_mark(database, plan)

    expect_integrity_error(
        database,
        """UPDATE legacy_notification_action_operations_v1
           SET completed_at_ms=completed_at_ms+1 WHERE operation_id=?1""",
        (plan.operation_id,),
        message="frame_legacy_notification_operation_immutable_v1",
    )
    expect_integrity_error(
        database,
        "DELETE FROM legacy_notification_action_operations_v1 WHERE operation_id=?1",
        (plan.operation_id,),
        message="frame_legacy_notification_operation_immutable_v1",
    )
    for table, marker in (
        (
            "legacy_notification_action_receipts_v1",
            "frame_legacy_notification_receipt_immutable_v1",
        ),
        (
            "legacy_notification_action_effects_v1",
            "frame_legacy_notification_effect_immutable_v1",
        ),
        (
            "legacy_notification_action_audit_events_v1",
            "frame_legacy_notification_audit_immutable_v1",
        ),
    ):
        expect_integrity_error(
            database,
            f"UPDATE {table} SET operation_id=operation_id WHERE operation_id=?1",
            (plan.operation_id,),
            message=marker,
        )
        expect_integrity_error(
            database,
            f"DELETE FROM {table} WHERE operation_id=?1",
            (plan.operation_id,),
            message=marker,
        )
    expect_integrity_error(
        database,
        """UPDATE legacy_notification_action_proof_consumptions_v1
           SET outcome=outcome WHERE mutation_grant_id=?1""",
        (grant.grant_id,),
        message="frame_legacy_notification_proof_immutable_v1",
    )
    expect_integrity_error(
        database,
        """DELETE FROM legacy_notification_action_proof_consumptions_v1
           WHERE mutation_grant_id=?1""",
        (grant.grant_id,),
        message="frame_legacy_notification_proof_immutable_v1",
    )

    # Tenant kind, nullable organization, actor, and action cannot be mixed.
    expect_integrity_error(
        database,
        SQL["operation_claim"],
        (
            uuid7_fixture(90_000),
            "actor",
            ACTOR,
            ORGANIZATION,
            ACTOR,
            MARK_ACTION,
            digest("invalid-scope-key"),
            digest("invalid-scope-request"),
            NOW_MS,
        ),
    )
    expect_integrity_error(
        database,
        SQL["operation_claim"],
        (
            uuid7_fixture(90_001),
            "organization",
            ORGANIZATION,
            ORGANIZATION,
            ACTOR,
            MARK_ACTION,
            plan.idempotency_key_digest,
            digest("same-key-different-request"),
            NOW_MS,
        ),
    )
    expect_integrity_error(
        database,
        """INSERT INTO legacy_notification_action_assertions_v1(
             operation_id,assertion_kind,expected_count,actual_count
           ) VALUES (?1,'mark_updated',1,0)""",
        (uuid7_fixture(90_002),),
    )
    assert database.execute("PRAGMA foreign_key_check").fetchall() == []
    assert database.execute("PRAGMA quick_check").fetchone()[0] == "ok"
    database.close()


def test_static_query_bounds_scopes_and_fail_closed_runtime() -> None:
    required_queries = {
        "assertion_cleanup",
        "audit_insert",
        "browser_grant_assert",
        "browser_grant_delete_returning",
        "changes_assert",
        "clock_now",
        "durable_receipt_assert",
        "effect_insert",
        "mark_authority_assert",
        "mark_authority_snapshot",
        "mark_matching_count",
        "mark_out_of_scope_assert",
        "mark_postcondition_assert",
        "mark_precondition_assert",
        "mark_update",
        "operation_by_key",
        "operation_claim",
        "operation_complete",
        "preferences_authority_assert",
        "preferences_other_actor_assert",
        "preferences_postcondition_assert",
        "preferences_snapshot",
        "preferences_update",
        "proof_insert",
        "receipt_insert",
    }
    assert set(SQL) == required_queries
    combined_sql = "\n".join(SQL.values())
    assert "SELECT *" not in combined_sql.upper()
    assert "legacy_api_execution_operations_v1" not in combined_sql
    assert "authenticated_web_action_operations_v1" not in combined_sql

    for name in (
        "operation_by_key",
        "mark_authority_snapshot",
        "preferences_snapshot",
    ):
        assert "LIMIT 2" in SQL[name]
    assert "LIMIT 1" in SQL["mark_matching_count"]
    assert "LIMIT 1" in SQL["clock_now"]
    for token in (
        "operation.tenant_kind = ?1",
        "operation.tenant_id = ?2",
        "operation.actor_id = ?3",
        "operation.action = ?4",
        "operation.idempotency_key_digest = ?5",
    ):
        assert token in SQL["operation_by_key"]
    for token in (
        "organization_id = ?2",
        "recipient_user_id = ?3",
        "(?4 IS NULL OR id = ?4)",
        "last_operation_id = ?1",
    ):
        assert token in SQL["mark_update"]
    assert "RETURNING id AS mutation_grant_id" in SQL[
        "browser_grant_delete_returning"
    ]
    for token in (
        "session_row.client_kind = 'browser'",
        "session_row.state = 'active'",
        "actor.status = 'active'",
        "actor.deleted_at_ms IS NULL",
        "session_row.generation = grant_row.generation",
        "session_row.session_version = identity_row.session_version",
    ):
        assert token in SQL["browser_grant_assert"]
    for token in (
        "notification_preferences_revision = ?3",
        "preferences_json IS ?4",
        "preferences_json = ?5",
        "notification_preferences_last_operation_id = ?1",
    ):
        assert token in SQL["preferences_update"]
    assert "u.id <> ?2" in SQL["preferences_other_actor_assert"]
    assert "proof.mutation_grant_id = ?21" in SQL["durable_receipt_assert"]
    assert "proof.session_id = ?22" in SQL["durable_receipt_assert"]
    assert "proof.outcome = ?23" in SQL["durable_receipt_assert"]

    runtime = RUNTIME.read_text(encoding="utf-8")
    application = APPLICATION.read_text(encoding="utf-8")
    for name in required_queries:
        assert f"legacy_notification_actions/{name}.sql" in runtime
    for token in (
        "frame.legacy-notification.operation-key.v1",
        ".batch(statements)",
        "decode_consumed_proof",
        "Value::Null => Map::new()",
        'skip_serializing_if = "Option::is_none"',
        "keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()))",
        "LegacyNotificationAtomicErrorV1::Unavailable",
        "LegacyNotificationAtomicErrorV1::Corrupt",
        "LegacyNotificationAtomicErrorV1::Conflict",
        "LegacyNotificationAtomicErrorV1::InFlight",
    ):
        assert token in runtime
    for forbidden in (
        "legacy_api_execution_operations_v1",
        "authenticated_web_action_operations_v1",
        "../queries/api_workflow/",
    ):
        assert forbidden not in runtime
    assert "matching_before == matched_count" in application
    assert "updated_rows == matched_count" in application
    assert "out_of_scope_updated_rows == 0" in application
    assert "preserved_before == preserved_after" in application

    migration = (
        MIGRATION_ROOT / "0038_legacy_notification_actions_expand.sql"
    ).read_text(encoding="utf-8")
    assert "ALTER TABLE notifications ADD COLUMN last_operation_id" not in migration
    assert (
        "UNIQUE (tenant_kind, tenant_id, actor_id, action, idempotency_key_digest)"
        in migration
    )
    assert "notifications(last_operation_id, organization_id, recipient_user_id)" in migration
    assert "users(notification_preferences_last_operation_id, id)" in migration
    assert "frame_legacy_notification_operation_immutable_v1" in migration
    assert "frame_legacy_notification_receipt_immutable_v1" in migration


TESTS = (
    test_full_migration_chain_and_expand_defaults,
    test_selected_and_bulk_mark_are_exactly_scoped,
    test_preferences_merge_preserves_siblings_and_optional_absence,
    test_replay_conflict_in_flight_and_one_use_proof,
    test_preferences_stale_snapshot_and_malformed_authority_fail_closed,
    test_failure_paths_are_atomic_and_fail_closed,
    test_concurrent_same_key_has_one_mutation_winner,
    test_typed_journal_constraints_and_immutability,
    test_static_query_bounds_scopes_and_fail_closed_runtime,
)


def run() -> dict[str, object]:
    for test in TESTS:
        test()
    return {
        "schema_version": "frame.legacy-notification-actions-sqlite-conformance.v1",
        "provider": "local_sqlite",
        "expand_migration": "0038_legacy_notification_actions_expand.sql",
        "full_expand_chain_applied": True,
        "tests_passed": len(TESTS),
        "actions": ["mark_as_read", "update_preferences"],
        "checked_in_queries_executed": len(SQL),
        "live_actor_and_active_tenant_authority": True,
        "selected_absent_foreign_non_disclosure": True,
        "bulk_mark_exact_actor_tenant_scope": True,
        "database_owned_read_timestamp": True,
        "actor_global_preferences": True,
        "latest_json_merge_preserves_siblings": True,
        "pause_anon_views_absence_preserved": True,
        "browser_grant_one_use": True,
        "receipt_effect_audit_proof_atomic": True,
        "same_key_exact_replay": True,
        "same_key_conflict_consumes_proof": True,
        "same_key_in_flight_consumes_proof": True,
        "race_applied": 1,
        "race_replayed": 1,
        "rollback_and_fail_closed": True,
        "bounded_sql": True,
        "durable_journal_immutable": True,
        "generic_legacy_journal_rows": 0,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    report = run()
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(
            json.dumps(report, indent=2) + "\n", encoding="utf-8"
        )
    print(
        "legacy notification-actions SQLite conformance: "
        f"{report['tests_passed']} passed; migration, scoped marks, "
        "preference merge, atomic proof/journal, replay/conflict/in-flight/race, "
        "rollback, bounds, and immutability verified"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
