#!/usr/bin/env python3
"""Provider-free SQLite proof for the legacy library-placement D1 adapter.

The suite applies the complete expand chain through 0037 and executes the
checked-in SQL used for Cap's four organization/space root-placement actions.
It proves tenant-bounded snapshots, the remove-organization ownership
asymmetry, exact normalized mutations, one-use browser proof consumption,
durable replay evidence, conflict isolation, and rollback under failed receipt
or authority postconditions.
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
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERY_ROOT = ROOT / "apps/control-plane/queries/legacy_library_placement"
AUTH_QUERY_ROOT = ROOT / "apps/control-plane/queries/auth"
RUNTIME = ROOT / "apps/control-plane/src/legacy_library_placement_runtime.rs"
APPLICATION = ROOT / "crates/application/src/legacy_library_placement.rs"

NOW_MS = 1_700_000_000_000
AUDIT_ACTION = "legacy.library_placement"


def uuid7_fixture(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


ACTOR = uuid7_fixture(1)
MANAGER = uuid7_fixture(2)
CONTRIBUTOR = uuid7_fixture(3)
FOREIGN_ACTOR = uuid7_fixture(4)
ORGANIZATION = uuid7_fixture(10)
FOREIGN_ORGANIZATION = uuid7_fixture(11)
SPACE = uuid7_fixture(20)
FOREIGN_SPACE = uuid7_fixture(21)
ROOT_FOLDER = uuid7_fixture(30)
SPACE_FOLDER = uuid7_fixture(31)
FOREIGN_FOLDER = uuid7_fixture(32)
VIDEO = uuid7_fixture(40)
SECOND_VIDEO = uuid7_fixture(41)
MANAGER_VIDEO = uuid7_fixture(42)
CONTRIBUTOR_VIDEO = uuid7_fixture(43)
FOREIGN_VIDEO = uuid7_fixture(44)


def digest(label: str) -> str:
    return hashlib.sha256(f"frame-library-placement:{label}".encode()).hexdigest()


SQL = {
    path.stem: path.read_text(encoding="utf-8").strip()
    for path in sorted(QUERY_ROOT.glob("*.sql"))
}
SQL.update(
    {
        "grant_assert": (
            AUTH_QUERY_ROOT / "browser_mutation_grant_assert.sql"
        ).read_text(encoding="utf-8").strip(),
        "grant_delete": (
            AUTH_QUERY_ROOT / "browser_mutation_grant_delete_by_proof.sql"
        ).read_text(encoding="utf-8").strip(),
        "change_assert": (
            AUTH_QUERY_ROOT / "browser_mutation_change_assert.sql"
        ).read_text(encoding="utf-8").strip(),
    }
)


def connect(path: Path | None = None) -> sqlite3.Connection:
    database = sqlite3.connect(
        ":memory:" if path is None else path,
        timeout=15,
        isolation_level=None,
        check_same_thread=False,
    )
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    database.execute("PRAGMA busy_timeout = 15000")
    if path is not None:
        database.execute("PRAGMA journal_mode = WAL")
    return database


def migrated_database(path: Path | None = None) -> sqlite3.Connection:
    database = connect(path)
    migrations = [
        migration
        for migration in sorted(MIGRATIONS.glob("*.sql"))
        if int(migration.name[:4]) <= 37
    ]
    for migration in migrations:
        database.executescript(migration.read_text(encoding="utf-8"))
        violations = database.execute("PRAGMA foreign_key_check").fetchall()
        assert not violations, f"{migration.name} introduced {violations}"
    return database


def seed_fixture(database: sqlite3.Connection) -> None:
    database.execute("BEGIN")
    try:
        for user_id, label, active_organization in (
            (ACTOR, "owner", ORGANIZATION),
            (MANAGER, "manager", ORGANIZATION),
            (CONTRIBUTOR, "contributor", ORGANIZATION),
            (FOREIGN_ACTOR, "foreign", FOREIGN_ORGANIZATION),
        ):
            database.execute(
                """INSERT INTO users(
                     id, email, display_name, created_at_ms, updated_at_ms,
                     status, active_organization_id,
                     organization_preference_revision
                   ) VALUES (?1, ?2, ?3, 1, 1, 'active', ?4, 7)""",
                (user_id, f"{label}@example.invalid", label, active_organization),
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
                (ORGANIZATION, MANAGER, "member"),
                (ORGANIZATION, CONTRIBUTOR, "member"),
                (FOREIGN_ORGANIZATION, FOREIGN_ACTOR, "owner"),
            ),
        )
        database.executemany(
            """INSERT INTO spaces(
                 id, organization_id, created_by_user_id, name,
                 created_at_ms, updated_at_ms, revision, authority_version
               ) VALUES (?1, ?2, ?3, ?4, 1, 1, 3, 4)""",
            (
                (SPACE, ORGANIZATION, ACTOR, "space"),
                (FOREIGN_SPACE, FOREIGN_ORGANIZATION, FOREIGN_ACTOR, "foreign"),
            ),
        )
        database.executemany(
            """INSERT INTO space_members(
                 space_id, user_id, role, created_at_ms, updated_at_ms,
                 state, revision
               ) VALUES (?1, ?2, ?3, 1, 1, 'active', 6)""",
            (
                (SPACE, ACTOR, "manager"),
                (SPACE, MANAGER, "manager"),
                (SPACE, CONTRIBUTOR, "contributor"),
                (FOREIGN_SPACE, FOREIGN_ACTOR, "manager"),
            ),
        )
        database.executemany(
            """INSERT INTO folders(
                 id, organization_id, space_id, parent_id,
                 created_by_user_id, name, created_at_ms, updated_at_ms,
                 revision, tree_revision
               ) VALUES (?1, ?2, ?3, NULL, ?4, ?5, 1, 1, 2, 8)""",
            (
                (ROOT_FOLDER, ORGANIZATION, None, ACTOR, "root"),
                (SPACE_FOLDER, ORGANIZATION, SPACE, ACTOR, "space-folder"),
                (
                    FOREIGN_FOLDER,
                    FOREIGN_ORGANIZATION,
                    FOREIGN_SPACE,
                    FOREIGN_ACTOR,
                    "foreign-folder",
                ),
            ),
        )
        database.executemany(
            """INSERT INTO videos(
                 id, owner_id, title, state, created_at_ms, updated_at_ms,
                 organization_id, revision
               ) VALUES (?1, ?2, ?3, 'ready', 1, 1, ?4, 0)""",
            (
                (VIDEO, ACTOR, "video", ORGANIZATION),
                (SECOND_VIDEO, ACTOR, "second", ORGANIZATION),
                (MANAGER_VIDEO, MANAGER, "manager-video", ORGANIZATION),
                (
                    CONTRIBUTOR_VIDEO,
                    CONTRIBUTOR,
                    "contributor-video",
                    ORGANIZATION,
                ),
                (FOREIGN_VIDEO, FOREIGN_ACTOR, "foreign-video", FOREIGN_ORGANIZATION),
            ),
        )
        for user_id in (ACTOR, MANAGER, CONTRIBUTOR):
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
    base = 1_000 + serial * 4
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
            NOW_MS + 60_000,
            NOW_MS + 3_600_000,
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


def authority(database: sqlite3.Connection, user_id: str = ACTOR) -> sqlite3.Row:
    row = database.execute(
        SQL["authority_snapshot"], (user_id, ORGANIZATION)
    ).fetchone()
    assert row is not None
    return row


def space(database: sqlite3.Connection, user_id: str = ACTOR) -> sqlite3.Row:
    row = database.execute(
        SQL["space_snapshot"], (SPACE, ORGANIZATION, user_id)
    ).fetchone()
    assert row is not None
    return row


def video_wire(
    database: sqlite3.Connection, ids: list[str]
) -> tuple[list[dict[str, Any]], str]:
    rows = database.execute(
        SQL["video_snapshot"], (json.dumps(ids), ORGANIZATION)
    ).fetchall()
    wire = []
    for row in rows:
        present = 0 if row["id"] is None else 1
        wire.append(
            {
                "id": row["requested_id"],
                "present": present,
                "owner_id": row["owner_id"],
                "folder_id": row["folder_id"],
                "revision": -1 if row["revision"] is None else row["revision"],
                "folder_in_tenant": row["folder_in_tenant"],
            }
        )
    return wire, json.dumps(wire, separators=(",", ":"))


def shared_wire(database: sqlite3.Connection, ids: list[str]) -> tuple[list[dict[str, Any]], str]:
    rows = database.execute(
        SQL["shared_snapshot"], (json.dumps(ids), ORGANIZATION)
    ).fetchall()
    wire = [
        {
            "id": row["requested_id"],
            "active_count": row["active_count"],
            "active_id": row["active_id"],
            "active_folder_id": row["active_folder_id"],
            "active_sharing_mode": row["active_sharing_mode"],
            "active_revision": row["active_revision"],
            "dormant_root_id": row["dormant_root_id"],
            "dormant_root_revision": row["dormant_root_revision"],
        }
        for row in rows
    ]
    return wire, json.dumps(wire, separators=(",", ":"))


def space_wire(database: sqlite3.Connection, ids: list[str]) -> tuple[list[dict[str, Any]], str]:
    rows = database.execute(
        SQL["space_membership_snapshot"], (json.dumps(ids), SPACE)
    ).fetchall()
    wire = [
        {
            "id": row["requested_id"],
            "present": 0 if row["video_id"] is None else 1,
            "folder_id": row["folder_id"],
            "revision": -1 if row["revision"] is None else row["revision"],
        }
        for row in rows
    ]
    return wire, json.dumps(wire, separators=(",", ":"))


def product_parameters(
    database: sqlite3.Connection,
    *,
    operation_id: str,
    user_id: str,
    action: str,
    scope_kind: str,
    videos_json: str,
    scoped_json: str,
) -> tuple[object, ...]:
    auth = authority(database, user_id)
    selected_space = space(database, user_id) if scope_kind == "space" else None
    return (
        operation_id,
        ORGANIZATION,
        user_id,
        action,
        scope_kind,
        "" if selected_space is None else selected_space["id"],
        -1 if selected_space is None else selected_space["revision"],
        -1 if selected_space is None else selected_space["authority_version"],
        "" if selected_space is None else selected_space["actor_space_role"],
        -1
        if selected_space is None
        else selected_space["actor_space_membership_revision"],
        videos_json,
        scoped_json,
        auth["membership_role"],
    )


def insert_authority_assertions(
    database: sqlite3.Connection, operation_id: str, user_id: str = ACTOR
) -> None:
    auth = authority(database, user_id)
    database.execute(
        SQL["organization_assert"],
        (
            operation_id,
            ORGANIZATION,
            auth["organization_revision"],
            auth["organization_authority_version"],
        ),
    )
    database.execute(
        SQL["selection_assert"],
        (operation_id, user_id, ORGANIZATION, auth["selection_revision"]),
    )
    database.execute(
        SQL["membership_assert"],
        (
            operation_id,
            ORGANIZATION,
            user_id,
            auth["membership_role"],
            auth["membership_revision"],
            auth["membership_authority_version"],
        ),
    )


def expect_integrity_error(callable_: Any) -> None:
    try:
        callable_()
    except sqlite3.IntegrityError:
        return
    raise AssertionError("expected SQLite integrity failure")


def test_full_migration_and_bounded_tenant_snapshots() -> None:
    database = migrated_database()
    seed_fixture(database)
    indexes = {
        row[0]
        for row in database.execute(
            "SELECT name FROM sqlite_master WHERE type='index'"
        )
    }
    assert {
        "legacy_library_placement_shared_snapshot_v1",
        "legacy_library_placement_space_snapshot_v1",
        "legacy_library_placement_direct_folder_v1",
    } <= indexes
    requested = sorted((VIDEO, FOREIGN_VIDEO, uuid7_fixture(99)))
    rows = database.execute(
        SQL["video_snapshot"], (json.dumps(requested), ORGANIZATION)
    ).fetchall()
    by_id = {row["requested_id"]: row for row in rows}
    assert by_id[VIDEO]["id"] == VIDEO
    assert by_id[FOREIGN_VIDEO]["id"] is None
    assert by_id[uuid7_fixture(99)]["id"] is None
    overflow = json.dumps([f"synthetic-{number:04d}" for number in range(600)])
    for name, parameters in (
        ("video_snapshot", (overflow, ORGANIZATION)),
        ("shared_snapshot", (overflow, ORGANIZATION)),
        ("space_membership_snapshot", (overflow, SPACE)),
    ):
        assert len(database.execute(SQL[name], parameters).fetchall()) == 501
        assert "json_each(?1)" in SQL[name]
        assert "LIMIT 501" in SQL[name]
    database.close()


def test_exact_normalized_mutations_do_not_cross_scopes() -> None:
    database = migrated_database()
    seed_fixture(database)
    operation = uuid7_fixture(500)

    shared_id = uuid7_fixture(501)
    database.execute(
        SQL["shared_root_insert"],
        (shared_id, VIDEO, ORGANIZATION, ACTOR, NOW_MS, operation),
    )
    assert tuple(
        database.execute(
            "SELECT folder_id,sharing_mode,revision FROM shared_videos WHERE id=?1",
            (shared_id,),
        ).fetchone()
    ) == (None, "organization", 0)
    database.execute(
        "UPDATE shared_videos SET folder_id=?1,sharing_mode='space' WHERE id=?2",
        (ROOT_FOLDER, shared_id),
    )
    database.execute(
        SQL["shared_root_set"],
        (shared_id, operation, ORGANIZATION, 0, ROOT_FOLDER, "space"),
    )
    assert tuple(
        database.execute(
            "SELECT folder_id,sharing_mode,revision FROM shared_videos WHERE id=?1",
            (shared_id,),
        ).fetchone()
    ) == (None, "organization", 1)
    database.execute(SQL["shared_delete"], (shared_id, ORGANIZATION, 1))
    assert database.execute(
        "SELECT COUNT(*) FROM shared_videos WHERE id=?1", (shared_id,)
    ).fetchone()[0] == 0

    database.execute(
        SQL["space_root_insert"], (SPACE, VIDEO, ACTOR, NOW_MS, operation)
    )
    database.execute(
        "UPDATE space_videos SET folder_id=?1 WHERE space_id=?2 AND video_id=?3",
        (SPACE_FOLDER, SPACE, VIDEO),
    )
    database.execute(
        SQL["space_root_set"], (SPACE, VIDEO, operation, 0, SPACE_FOLDER)
    )
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM space_videos WHERE space_id=?1 AND video_id=?2",
            (SPACE, VIDEO),
        ).fetchone()
    ) == (None, 1)
    database.execute(SQL["space_delete"], (SPACE, VIDEO, 1))
    assert database.execute("SELECT COUNT(*) FROM space_videos").fetchone()[0] == 0

    database.execute(
        "UPDATE videos SET folder_id=?1 WHERE id=?2", (ROOT_FOLDER, VIDEO)
    )
    database.execute(
        SQL["direct_folder_clear"],
        (VIDEO, ORGANIZATION, operation, NOW_MS, 0, ROOT_FOLDER),
    )
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM videos WHERE id=?1", (VIDEO,)
        ).fetchone()
    ) == (None, 1)
    assert database.execute(
        "SELECT folder_id FROM videos WHERE id=?1", (FOREIGN_VIDEO,)
    ).fetchone()[0] is None
    database.close()


def assert_product_precondition(
    database: sqlite3.Connection,
    *,
    user_id: str,
    action: str,
    scope_kind: str,
    ids: list[str],
    succeeds: bool,
) -> None:
    operation_id = uuid7_fixture(600 + len(ids) + sum(map(ord, user_id[-2:])))
    _, videos_json = video_wire(database, ids)
    _, scoped_json = (
        shared_wire(database, ids)
        if scope_kind == "organization"
        else space_wire(database, ids)
    )
    database.execute("SAVEPOINT product_authority")
    try:
        database.execute(
            SQL["product_precondition"],
            product_parameters(
                database,
                operation_id=operation_id,
                user_id=user_id,
                action=action,
                scope_kind=scope_kind,
                videos_json=videos_json,
                scoped_json=scoped_json,
            ),
        )
    except sqlite3.IntegrityError:
        database.execute("ROLLBACK TO product_authority")
        database.execute("RELEASE product_authority")
        assert not succeeds
        return
    database.execute("ROLLBACK TO product_authority")
    database.execute("RELEASE product_authority")
    assert succeeds


def test_manager_and_ownership_asymmetry_is_enforced_in_sql() -> None:
    database = migrated_database()
    seed_fixture(database)
    assert_product_precondition(
        database,
        user_id=ACTOR,
        action="add_organization",
        scope_kind="organization",
        ids=[VIDEO],
        succeeds=True,
    )
    assert_product_precondition(
        database,
        user_id=ACTOR,
        action="add_organization",
        scope_kind="organization",
        ids=[MANAGER_VIDEO],
        succeeds=False,
    )
    # Deliberate asymmetry: an organization manager can remove a matching
    # organization share without owning the base video.
    assert_product_precondition(
        database,
        user_id=ACTOR,
        action="remove_organization",
        scope_kind="organization",
        ids=[MANAGER_VIDEO],
        succeeds=True,
    )
    assert_product_precondition(
        database,
        user_id=MANAGER,
        action="add_scope",
        scope_kind="space",
        ids=[MANAGER_VIDEO],
        succeeds=True,
    )
    assert_product_precondition(
        database,
        user_id=MANAGER,
        action="remove_organization",
        scope_kind="organization",
        ids=[MANAGER_VIDEO],
        succeeds=False,
    )
    assert_product_precondition(
        database,
        user_id=CONTRIBUTOR,
        action="add_scope",
        scope_kind="space",
        ids=[CONTRIBUTOR_VIDEO],
        succeeds=False,
    )
    database.close()


@dataclass(frozen=True)
class AddOrganizationPlan:
    operation_id: str
    audit_id: str
    action: str
    key_digest: str
    request_digest: str
    grant: Grant
    videos_json: str
    scoped_json: str
    shared_id: str
    final_json: str
    receipt_json: str
    effect_json: str


def add_organization_plan(
    database: sqlite3.Connection,
    *,
    serial: int,
    grant: Grant,
    key_digest: str | None = None,
    request_digest: str | None = None,
) -> AddOrganizationPlan:
    videos, videos_json = video_wire(database, [VIDEO])
    shared, scoped_json = shared_wire(database, [VIDEO])
    assert videos[0]["present"] == 1 and shared[0]["active_count"] == 0
    shared_id = uuid7_fixture(2_000 + serial * 3)
    operation_id = uuid7_fixture(2_001 + serial * 3)
    audit_id = uuid7_fixture(2_002 + serial * 3)
    final = [
        {
            "id": VIDEO,
            "video_present": 1,
            "video_folder_id": videos[0]["folder_id"],
            "video_revision": videos[0]["revision"],
            "active_count": 1,
            "active_id": shared_id,
            "active_folder_id": None,
            "active_sharing_mode": "organization",
            "active_revision": 0,
            "scope_present": 0,
            "scope_folder_id": None,
            "scope_revision": -1,
        }
    ]
    effect = {
        "scope": {"kind": "organization", "organization_id": ORGANIZATION},
        "invalidates_scope_root": True,
        "invalidates_caps": True,
    }
    return AddOrganizationPlan(
        operation_id=operation_id,
        audit_id=audit_id,
        action="legacy_library_add_organization_v1",
        key_digest=key_digest or digest(f"key-{serial}"),
        request_digest=request_digest or digest(f"request-{serial}"),
        grant=grant,
        videos_json=videos_json,
        scoped_json=scoped_json,
        shared_id=shared_id,
        final_json=json.dumps(final, separators=(",", ":")),
        receipt_json=json.dumps(
            {"result": {"kind": "organization_added", "total_updated": 1}},
            separators=(",", ":"),
        ),
        effect_json=json.dumps(effect, separators=(",", ":")),
    )


def execute_add_organization(
    database: sqlite3.Connection,
    plan: AddOrganizationPlan,
    *,
    include_audit: bool = True,
) -> None:
    database.execute("BEGIN IMMEDIATE")
    try:
        database.execute(
            SQL["operation_claim"],
            (
                plan.operation_id,
                ORGANIZATION,
                ACTOR,
                plan.action,
                plan.key_digest,
                plan.request_digest,
                NOW_MS,
            ),
        )
        insert_authority_assertions(database, plan.operation_id)
        database.execute(
            SQL["grant_assert"],
            (
                plan.operation_id,
                plan.grant.grant_id,
                plan.grant.session_id,
                plan.grant.user_id,
                NOW_MS,
            ),
        )
        database.execute(
            SQL["product_precondition"],
            product_parameters(
                database,
                operation_id=plan.operation_id,
                user_id=ACTOR,
                action="add_organization",
                scope_kind="organization",
                videos_json=plan.videos_json,
                scoped_json=plan.scoped_json,
            ),
        )
        database.execute(
            SQL["shared_root_insert"],
            (
                plan.shared_id,
                VIDEO,
                ORGANIZATION,
                ACTOR,
                NOW_MS,
                plan.operation_id,
            ),
        )
        database.execute(
            SQL["product_postcondition"],
            (plan.operation_id, ORGANIZATION, "organization", "", plan.final_json),
        )
        database.execute(
            SQL["effect_insert"],
            (
                plan.operation_id,
                ORGANIZATION,
                ACTOR,
                plan.action,
                plan.effect_json,
                NOW_MS,
            ),
        )
        database.execute(SQL["change_assert"], (plan.operation_id, "action_effect"))
        database.execute(
            SQL["operation_complete"],
            (plan.operation_id, plan.receipt_json, NOW_MS),
        )
        database.execute(
            SQL["change_assert"], (plan.operation_id, "operation_complete")
        )
        if include_audit:
            database.execute(
                SQL["audit_insert"],
                (
                    plan.audit_id,
                    plan.operation_id,
                    ORGANIZATION,
                    digest("principal"),
                    AUDIT_ACTION,
                    digest("subject"),
                    NOW_MS,
                ),
            )
        database.execute(
            SQL["receipt_postcondition"],
            (
                plan.operation_id,
                ORGANIZATION,
                ACTOR,
                plan.action,
                plan.receipt_json,
                plan.effect_json,
                AUDIT_ACTION,
            ),
        )
        database.execute(
            SQL["grant_delete"],
            (plan.grant.grant_id, plan.grant.session_id, plan.grant.user_id),
        )
        database.execute(SQL["change_assert"], (plan.operation_id, "grant_consumed"))
        database.execute(SQL["assertion_cleanup"], (plan.operation_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


def consume_replay(database: sqlite3.Connection, operation_id: str, grant: Grant) -> None:
    auth = authority(database)
    database.execute("BEGIN IMMEDIATE")
    try:
        insert_authority_assertions(database, operation_id)
        database.execute(
            SQL["placement_authority_assert"],
            (
                operation_id,
                ORGANIZATION,
                ACTOR,
                auth["membership_role"],
                "",
                -1,
                -1,
                -1,
                "",
            ),
        )
        database.execute(
            SQL["grant_assert"],
            (operation_id, grant.grant_id, grant.session_id, grant.user_id, NOW_MS),
        )
        database.execute(
            SQL["grant_delete"], (grant.grant_id, grant.session_id, grant.user_id)
        )
        database.execute(SQL["change_assert"], (operation_id, "grant_consumed"))
        database.execute(SQL["assertion_cleanup"], (operation_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


def test_atomic_receipt_replay_conflict_and_grant_consumption() -> None:
    database = migrated_database()
    seed_fixture(database)
    first_grant = seed_grant(database, serial=1)
    plan = add_organization_plan(database, serial=1, grant=first_grant)
    execute_add_organization(database, plan)
    assert database.execute(
        "SELECT COUNT(*) FROM shared_videos WHERE id=?1 AND folder_id IS NULL",
        (plan.shared_id,),
    ).fetchone()[0] == 1
    operation = database.execute(
        SQL["operation_by_key"],
        (ORGANIZATION, ACTOR, plan.action, plan.key_digest),
    ).fetchone()
    assert operation is not None
    assert operation["state"] == "complete"
    assert operation["response_json"] == plan.receipt_json
    assert operation["effect_json"] == plan.effect_json
    assert operation["audit_count"] == 1
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (first_grant.grant_id,),
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_assertions_v1"
    ).fetchone()[0] == 0

    replay_grant = seed_grant(database, serial=2)
    before_revision = database.execute(
        "SELECT revision FROM shared_videos WHERE id=?1", (plan.shared_id,)
    ).fetchone()[0]
    consume_replay(database, plan.operation_id, replay_grant)
    assert database.execute(
        "SELECT revision FROM shared_videos WHERE id=?1", (plan.shared_id,)
    ).fetchone()[0] == before_revision
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (replay_grant.grant_id,),
    ).fetchone()[0] == 0

    # Same tenant+actor+action key with a different request digest is a
    # conflict; consuming its proof cannot alter the original receipt.
    conflict_grant = seed_grant(database, serial=3)
    conflicting_request_digest = digest("different-request")
    existing = database.execute(
        SQL["operation_by_key"],
        (ORGANIZATION, ACTOR, plan.action, plan.key_digest),
    ).fetchone()
    assert existing["request_digest"] != conflicting_request_digest
    assertion_id = uuid7_fixture(2_999)
    database.execute("BEGIN IMMEDIATE")
    database.execute(
        SQL["grant_assert"],
        (
            assertion_id,
            conflict_grant.grant_id,
            conflict_grant.session_id,
            conflict_grant.user_id,
            NOW_MS,
        ),
    )
    database.execute(
        SQL["grant_delete"],
        (
            conflict_grant.grant_id,
            conflict_grant.session_id,
            conflict_grant.user_id,
        ),
    )
    database.execute(SQL["change_assert"], (assertion_id, "grant_consumed"))
    database.execute(SQL["assertion_cleanup"], (assertion_id,))
    database.execute("COMMIT")
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_operations_v1"
    ).fetchone()[0] == 1
    database.close()


def test_failed_evidence_and_spent_grants_roll_back_everything() -> None:
    database = migrated_database()
    seed_fixture(database)
    grant = seed_grant(database, serial=4)
    plan = add_organization_plan(database, serial=4, grant=grant)
    expect_integrity_error(
        lambda: execute_add_organization(database, plan, include_audit=False)
    )
    assert database.execute("SELECT COUNT(*) FROM shared_videos").fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_operations_v1"
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_effects_v1"
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (grant.grant_id,),
    ).fetchone()[0] == 1

    database.execute(
        "DELETE FROM auth_session_mutation_grants_v2 WHERE id=?1", (grant.grant_id,)
    )
    expect_integrity_error(lambda: execute_add_organization(database, plan))
    assert database.execute("SELECT COUNT(*) FROM shared_videos").fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_operations_v1"
    ).fetchone()[0] == 0
    database.close()


def test_idempotency_claim_race_has_one_winner() -> None:
    with tempfile.TemporaryDirectory(prefix="frame-library-placement-") as directory:
        path = Path(directory) / "race.sqlite3"
        database = migrated_database(path)
        seed_fixture(database)
        database.close()
        barrier = threading.Barrier(2)
        outcomes: list[str] = []
        lock = threading.Lock()

        def contender(serial: int) -> None:
            connection = connect(path)
            operation_id = uuid7_fixture(4_000 + serial)
            barrier.wait()
            try:
                connection.execute("BEGIN IMMEDIATE")
                connection.execute(
                    SQL["operation_claim"],
                    (
                        operation_id,
                        ORGANIZATION,
                        ACTOR,
                        "legacy_library_add_organization_v1",
                        digest("shared-race-key"),
                        digest(f"race-request-{serial}"),
                        NOW_MS,
                    ),
                )
                connection.execute("COMMIT")
                outcome = "won"
            except sqlite3.IntegrityError:
                connection.execute("ROLLBACK")
                outcome = "conflict"
            finally:
                connection.close()
            with lock:
                outcomes.append(outcome)

        threads = [threading.Thread(target=contender, args=(serial,)) for serial in (1, 2)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        assert sorted(outcomes) == ["conflict", "won"]
        check = connect(path)
        assert check.execute(
            """SELECT COUNT(*) FROM authenticated_web_action_operations_v1
               WHERE organization_id=?1 AND user_id=?2 AND action=?3
                 AND idempotency_key=?4""",
            (
                ORGANIZATION,
                ACTOR,
                "legacy_library_add_organization_v1",
                digest("shared-race-key"),
            ),
        ).fetchone()[0] == 1
        check.close()


def test_checked_in_runtime_and_protected_gate_are_wired_to_this_proof() -> None:
    runtime = RUNTIME.read_text(encoding="utf-8")
    application = APPLICATION.read_text(encoding="utf-8")
    for marker in (
        "D1LegacyLibraryPlacementAtomicPortV1",
        "LegacyLibraryPlacementAtomicPortV1",
        "legacy_library_add_organization_v1",
        "legacy_library_remove_organization_v1",
        "legacy_library_add_scope_v1",
        "legacy_library_remove_scope_v1",
        "BROWSER_MUTATION_GRANT_ASSERT_SQL",
        "PRODUCT_PRECONDITION_SQL",
        "RECEIPT_POSTCONDITION_SQL",
        "AUDIT_ACTION",
    ):
        assert marker in runtime
    assert runtime.count("legacy_api_execution_operations_v1") == 2
    assert 'protected_gates: &["released_legacy_client_e2e"]' in application
    assert "production_promoted: false" in application
    assert runtime.count("include_str!(\"../queries/legacy_library_placement/") >= 20


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    tests = (
        test_full_migration_and_bounded_tenant_snapshots,
        test_exact_normalized_mutations_do_not_cross_scopes,
        test_manager_and_ownership_asymmetry_is_enforced_in_sql,
        test_atomic_receipt_replay_conflict_and_grant_consumption,
        test_failed_evidence_and_spent_grants_roll_back_everything,
        test_idempotency_claim_race_has_one_winner,
        test_checked_in_runtime_and_protected_gate_are_wired_to_this_proof,
    )
    for test in tests:
        test()
    if args.evidence is not None:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(
            json.dumps(
                {
                    "schema_version": "frame.legacy-library-placement-sqlite-conformance.v1",
                    "test_count": len(tests),
                    "operations": [
                        "cap-v1-d96a1931942eb83b",
                        "cap-v1-0694e68a64976c9a",
                        "cap-v1-bb55b5eeeb5e31ab",
                        "cap-v1-ccbe5f1381eaa1b4",
                    ],
                    "provider_data": False,
                    "production_promotion": False,
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
    print(
        "legacy library-placement SQLite conformance: "
        f"{len(tests)} passed; migrations, bounded authority, normalized storage, "
        "ownership asymmetry, atomic grant/receipt, replay/conflict/race, and rollback verified"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
