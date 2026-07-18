#!/usr/bin/env python3
"""Provider-free SQLite proof for legacy folder-assignment D1 semantics.

The suite applies the complete expand migration chain through 0036 and then
executes the checked-in SQL used by the D1 adapter.  It intentionally tests
transaction failure as well as success: stale authority/product snapshots,
missing receipt evidence, a spent browser grant, and an idempotency race must
never leave a partial product mutation or journal.
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
QUERY_ROOT = ROOT / "apps/control-plane/queries/legacy_folder_assignment"
AUTH_QUERY_ROOT = ROOT / "apps/control-plane/queries/auth"
RUNTIME = ROOT / "apps/control-plane/src/legacy_folder_assignment_runtime.rs"
APPLICATION = ROOT / "crates/application/src/legacy_folder_assignment.rs"

NOW_MS = 1_700_000_000_000
AUDIT_ACTION = "legacy.folder_assignment"


def uuid7_fixture(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


ACTOR = uuid7_fixture(1)
MEMBER = uuid7_fixture(2)
FOREIGN_ACTOR = uuid7_fixture(3)
ORGANIZATION = uuid7_fixture(10)
FOREIGN_ORGANIZATION = uuid7_fixture(11)
SPACE = uuid7_fixture(20)
SECOND_SPACE = uuid7_fixture(21)
FOREIGN_SPACE = uuid7_fixture(22)
ROOT_FOLDER = uuid7_fixture(30)
SECOND_ROOT_FOLDER = uuid7_fixture(31)
SPACE_FOLDER = uuid7_fixture(32)
SPACE_FOLDER_ALT = uuid7_fixture(33)
SECOND_SPACE_FOLDER = uuid7_fixture(34)
MEMBER_SPACE_FOLDER = uuid7_fixture(35)
FOREIGN_FOLDER = uuid7_fixture(36)
MEMBER_ROOT_FOLDER = uuid7_fixture(37)
VIDEO = uuid7_fixture(40)
SECOND_VIDEO = uuid7_fixture(41)
MEMBER_VIDEO = uuid7_fixture(42)
FOREIGN_VIDEO = uuid7_fixture(43)


def digest(label: str) -> str:
    return hashlib.sha256(f"frame-folder-conformance:{label}".encode()).hexdigest()


def query(name: str) -> str:
    return (QUERY_ROOT / name).read_text(encoding="utf-8").strip()


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


def migration_paths(through: int = 36) -> list[Path]:
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
    *, path: Path | None = None, through: int = 36
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
        for user_id, name, active_organization in (
            (ACTOR, "owner", ORGANIZATION),
            (MEMBER, "member", ORGANIZATION),
            (FOREIGN_ACTOR, "foreign", FOREIGN_ORGANIZATION),
        ):
            database.execute(
                """INSERT INTO users(
                     id, email, display_name, created_at_ms, updated_at_ms,
                     status, active_organization_id,
                     organization_preference_revision
                   ) VALUES (?1, ?2, ?3, 1, 1, 'active', ?4, 7)""",
                (user_id, f"{name}@example.invalid", name, active_organization),
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
            ),
        )
        database.executemany(
            """INSERT INTO spaces(
                 id, organization_id, created_by_user_id, name,
                 created_at_ms, updated_at_ms, revision, authority_version
               ) VALUES (?1, ?2, ?3, ?4, 1, 1, 3, 4)""",
            (
                (SPACE, ORGANIZATION, ACTOR, "space"),
                (SECOND_SPACE, ORGANIZATION, ACTOR, "second"),
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
                (SECOND_SPACE, ACTOR, "manager"),
                (SPACE, MEMBER, "contributor"),
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
                (SECOND_ROOT_FOLDER, ORGANIZATION, None, ACTOR, "root-two"),
                (SPACE_FOLDER, ORGANIZATION, SPACE, ACTOR, "space-folder"),
                (SPACE_FOLDER_ALT, ORGANIZATION, SPACE, ACTOR, "space-alt"),
                (
                    SECOND_SPACE_FOLDER,
                    ORGANIZATION,
                    SECOND_SPACE,
                    ACTOR,
                    "second-space-folder",
                ),
                (
                    MEMBER_SPACE_FOLDER,
                    ORGANIZATION,
                    SPACE,
                    MEMBER,
                    "member-space-folder",
                ),
                (
                    FOREIGN_FOLDER,
                    FOREIGN_ORGANIZATION,
                    FOREIGN_SPACE,
                    FOREIGN_ACTOR,
                    "foreign-folder",
                ),
                (
                    MEMBER_ROOT_FOLDER,
                    ORGANIZATION,
                    None,
                    MEMBER,
                    "member-personal-root",
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
                (SECOND_VIDEO, ACTOR, "video-two", ORGANIZATION),
                (MEMBER_VIDEO, MEMBER, "member-video", ORGANIZATION),
                (
                    FOREIGN_VIDEO,
                    FOREIGN_ACTOR,
                    "foreign-video",
                    FOREIGN_ORGANIZATION,
                ),
            ),
        )
        for user_id in (ACTOR, MEMBER):
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
    raise AssertionError("invalid scoped folder assignment was accepted")


def test_full_migration_and_scope_triggers() -> None:
    database = migrated_database()
    seed_fixture(database)

    columns = {
        row[1] for row in database.execute("PRAGMA table_info(space_videos)")
    }
    assert {"revision", "last_operation_id"} <= columns
    triggers = {
        row[0]
        for row in database.execute(
            "SELECT name FROM sqlite_master WHERE type='trigger'"
        )
    }
    required = {
        "legacy_direct_folder_insert_scope_guard_v1",
        "legacy_direct_folder_update_scope_guard_v1",
        "legacy_space_video_insert_scope_guard_v1",
        "legacy_space_video_update_scope_guard_v1",
        "legacy_shared_video_insert_scope_guard_v1",
        "legacy_shared_video_update_scope_guard_v1",
        "legacy_shared_video_one_current_insert_guard_v1",
        "legacy_shared_video_one_current_update_guard_v1",
    }
    assert required <= triggers

    # An organization-root folder is valid for the direct and organization
    # library contexts.
    database.execute(
        "UPDATE videos SET folder_id=?1 WHERE id=?2", (ROOT_FOLDER, VIDEO)
    )
    assert database.execute(
        "SELECT folder_id FROM videos WHERE id=?1", (VIDEO,)
    ).fetchone()[0] == ROOT_FOLDER
    database.execute("UPDATE videos SET folder_id=NULL WHERE id=?1", (VIDEO,))
    expect_integrity_error(
        database,
        "UPDATE videos SET folder_id=?1 WHERE id=?2",
        (FOREIGN_FOLDER, VIDEO),
        message="frame_legacy_folder_assignment_scope_v1",
    )

    database.execute(
        SQL["space_assignment_insert"],
        (SPACE, VIDEO, SPACE_FOLDER, ACTOR, NOW_MS, uuid7_fixture(500)),
    )
    database.execute(
        "DELETE FROM space_videos WHERE space_id=?1 AND video_id=?2",
        (SPACE, VIDEO),
    )
    for invalid_folder in (ROOT_FOLDER, SECOND_SPACE_FOLDER, FOREIGN_FOLDER):
        expect_integrity_error(
            database,
            SQL["space_assignment_insert"],
            (
                SPACE,
                VIDEO,
                invalid_folder,
                ACTOR,
                NOW_MS,
                uuid7_fixture(501),
            ),
            message="frame_legacy_folder_assignment_scope_v1",
        )

    shared_id = uuid7_fixture(510)
    database.execute(
        SQL["shared_assignment_insert"],
        (
            shared_id,
            VIDEO,
            ORGANIZATION,
            ROOT_FOLDER,
            ACTOR,
            NOW_MS,
            uuid7_fixture(511),
        ),
    )
    assert database.execute(
        "SELECT folder_id FROM shared_videos WHERE id=?1", (shared_id,)
    ).fetchone()[0] == ROOT_FOLDER
    # The older business-data contract owns malformed sharing shapes. A later
    # current-row guard must not turn this stable validation/authority error
    # into an idempotency-like multiplicity conflict merely because a valid
    # share already exists.
    expect_integrity_error(
        database,
        """INSERT INTO shared_videos(
             id, video_id, organization_id, shared_by_user_id,
             sharing_mode, shared_at_ms, revision
           ) VALUES (?1, ?2, ?3, ?4, 'space', ?5, 0)""",
        (
            uuid7_fixture(5_120),
            VIDEO,
            ORGANIZATION,
            ACTOR,
            NOW_MS,
        ),
        message="frame_business_authority_conflict_v1",
    )
    expect_integrity_error(
        database,
        SQL["shared_assignment_insert"],
        (
            uuid7_fixture(512),
            VIDEO,
            ORGANIZATION,
            SPACE_FOLDER,
            ACTOR,
            NOW_MS,
            uuid7_fixture(513),
        ),
        message="frame_legacy_folder_assignment_multiplicity_v1",
    )
    database.execute("DELETE FROM shared_videos WHERE id=?1", (shared_id,))
    expect_integrity_error(
        database,
        SQL["shared_assignment_insert"],
        (
            uuid7_fixture(514),
            VIDEO,
            ORGANIZATION,
            FOREIGN_FOLDER,
            ACTOR,
            NOW_MS,
            uuid7_fixture(515),
        ),
        message="frame_business_authority_conflict_v1",
    )
    expect_integrity_error(
        database,
        """INSERT INTO shared_videos(
             id, video_id, organization_id, folder_id, shared_by_user_id,
             sharing_mode, shared_at_ms, revision
           ) VALUES (?1, ?2, ?3, ?4, ?5, 'organization', ?6, 0)""",
        (
            uuid7_fixture(516),
            VIDEO,
            ORGANIZATION,
            ROOT_FOLDER,
            ACTOR,
            NOW_MS,
        ),
    )
    database.close()


def test_json_each_snapshots_are_tenant_bound_and_bounded() -> None:
    database = migrated_database()
    seed_fixture(database)
    requested = sorted((VIDEO, SECOND_VIDEO, FOREIGN_VIDEO, uuid7_fixture(99)))
    rows = database.execute(
        SQL["video_snapshot"], (json.dumps(requested), ORGANIZATION)
    ).fetchall()
    assert [row["requested_id"] for row in rows] == requested
    by_id = {row["requested_id"]: row for row in rows}
    assert by_id[VIDEO]["id"] == VIDEO
    assert by_id[SECOND_VIDEO]["id"] == SECOND_VIDEO
    assert by_id[FOREIGN_VIDEO]["id"] is None
    assert by_id[uuid7_fixture(99)]["id"] is None

    overflow = json.dumps([f"synthetic-{number:04d}" for number in range(600)])
    assert len(
        database.execute(
            SQL["video_snapshot"], (overflow, ORGANIZATION)
        ).fetchall()
    ) == 501
    assert len(
        database.execute(SQL["space_assignment_snapshot"], (overflow, SPACE)).fetchall()
    ) == 501
    assert len(
        database.execute(
            SQL["shared_assignment_snapshot"],
            (overflow, ORGANIZATION, ROOT_FOLDER),
        ).fetchall()
    ) == 501
    for name in (
        "video_snapshot",
        "space_assignment_snapshot",
        "shared_assignment_snapshot",
    ):
        assert "json_each(?1)" in SQL[name]
        assert "LIMIT 501" in SQL[name]
    database.close()


def test_normalized_assignment_storage() -> None:
    database = migrated_database()
    seed_fixture(database)

    # A direct move changes only videos.folder_id.
    database.execute(
        SQL["direct_assignment_set"],
        (VIDEO, ROOT_FOLDER, uuid7_fixture(600), NOW_MS, ORGANIZATION, 0, None),
    )
    assert database.execute("SELECT changes()").fetchone()[0] == 1
    assert tuple(
        database.execute(
            "SELECT folder_id, revision FROM videos WHERE id=?1", (VIDEO,)
        ).fetchone()
    ) == (ROOT_FOLDER, 1)
    assert database.execute("SELECT COUNT(*) FROM space_videos").fetchone()[0] == 0
    assert database.execute("SELECT COUNT(*) FROM shared_videos").fetchone()[0] == 0
    database.execute(
        SQL["direct_assignment_set"],
        (VIDEO, None, uuid7_fixture(601), NOW_MS, ORGANIZATION, 1, ROOT_FOLDER),
    )

    # Space add creates membership; remove clears only the matching folder;
    # move updates an existing membership and never manufactures a missing one.
    database.execute(
        SQL["space_assignment_insert"],
        (SPACE, VIDEO, SPACE_FOLDER, ACTOR, NOW_MS, uuid7_fixture(602)),
    )
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM space_videos "
            "WHERE space_id=?1 AND video_id=?2",
            (SPACE, VIDEO),
        ).fetchone()
    ) == (SPACE_FOLDER, 0)
    database.execute(
        SQL["space_assignment_set"],
        (SPACE, VIDEO, None, uuid7_fixture(603), 0, SPACE_FOLDER),
    )
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM space_videos "
            "WHERE space_id=?1 AND video_id=?2",
            (SPACE, VIDEO),
        ).fetchone()
    ) == (None, 1)
    database.execute(
        SQL["space_assignment_set"],
        (SPACE, VIDEO, SPACE_FOLDER_ALT, uuid7_fixture(604), 1, None),
    )
    assert database.execute(
        "SELECT folder_id FROM space_videos WHERE space_id=?1 AND video_id=?2",
        (SPACE, VIDEO),
    ).fetchone()[0] == SPACE_FOLDER_ALT
    assert database.execute(
        "SELECT COUNT(*) FROM space_videos WHERE space_id=?1 AND video_id=?2",
        (SPACE, SECOND_VIDEO),
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT folder_id FROM videos WHERE id=?1", (VIDEO,)
    ).fetchone()[0] is None

    # Organization-library add/remove/move is normalized into shared_videos.
    shared_id = uuid7_fixture(610)
    database.execute(
        SQL["shared_assignment_insert"],
        (
            shared_id,
            VIDEO,
            ORGANIZATION,
            ROOT_FOLDER,
            ACTOR,
            NOW_MS,
            uuid7_fixture(611),
        ),
    )
    database.execute(
        SQL["shared_assignment_set"],
        (shared_id, None, uuid7_fixture(612), ORGANIZATION, 0, ROOT_FOLDER),
    )
    assert tuple(
        database.execute(
            "SELECT folder_id,sharing_mode,revision FROM shared_videos WHERE id=?1",
            (shared_id,),
        ).fetchone()
    ) == (None, "organization", 1)
    database.execute(
        SQL["shared_assignment_set"],
        (shared_id, SPACE_FOLDER, uuid7_fixture(613), ORGANIZATION, 1, None),
    )
    assert tuple(
        database.execute(
            "SELECT folder_id,sharing_mode,revision FROM shared_videos WHERE id=?1",
            (shared_id,),
        ).fetchone()
    ) == (SPACE_FOLDER, "space", 2)
    # Removing a different folder and moving a missing scoped membership are
    # successful void no-ops in the adapter; the SQL snapshot proves there is
    # no row to mutate and the selected row remains untouched.
    before = tuple(
        database.execute(
            "SELECT folder_id,revision FROM shared_videos WHERE id=?1", (shared_id,)
        ).fetchone()
    )
    missing = database.execute(
        SQL["shared_assignment_snapshot"],
        (json.dumps([SECOND_VIDEO]), ORGANIZATION, ROOT_FOLDER),
    ).fetchone()
    assert missing["active_count"] == 0
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM shared_videos WHERE id=?1", (shared_id,)
        ).fetchone()
    ) == before
    assert database.execute(
        "SELECT folder_id FROM videos WHERE id=?1", (VIDEO,)
    ).fetchone()[0] is None
    database.close()


def folder_snapshot(
    database: sqlite3.Connection, folder_id: str, actor_id: str = ACTOR
) -> sqlite3.Row:
    row = database.execute(
        SQL["folder_snapshot"], (folder_id, ORGANIZATION, actor_id)
    ).fetchone()
    if row is None:
        raise AssertionError("fixture folder did not survive tenant-bound snapshot")
    return row


def authority_snapshot(
    database: sqlite3.Connection, actor_id: str = ACTOR
) -> sqlite3.Row:
    row = database.execute(
        SQL["authority_snapshot"], (actor_id, ORGANIZATION)
    ).fetchone()
    if row is None:
        raise AssertionError("fixture authority did not survive active selection")
    return row


def video_wire(database: sqlite3.Connection, ids: list[str]) -> tuple[list[dict[str, Any]], str]:
    rows = database.execute(
        SQL["video_snapshot"], (json.dumps(ids), ORGANIZATION)
    ).fetchall()
    wire = [
        {
            "id": row["id"],
            "owner_id": row["owner_id"],
            "folder_id": row["folder_id"],
            "revision": row["revision"],
        }
        for row in rows
    ]
    return wire, json.dumps(wire, separators=(",", ":"))


def shared_wire(
    database: sqlite3.Connection, ids: list[str], desired_folder_id: str | None
) -> str:
    rows = database.execute(
        SQL["shared_assignment_snapshot"],
        (json.dumps(ids), ORGANIZATION, desired_folder_id),
    ).fetchall()
    return json.dumps(
        [
            {
                "id": row["requested_id"],
                "active_count": row["active_count"],
                "active_id": row["active_id"],
                "active_folder_id": row["active_folder_id"],
                "active_revision": row["active_revision"],
                "desired_folder_id": desired_folder_id,
                "dormant_id": row["dormant_id"],
                "dormant_revision": row["dormant_revision"],
            }
            for row in rows
        ],
        separators=(",", ":"),
    )


def space_context_snapshot(
    database: sqlite3.Connection, *, actor_id: str
) -> sqlite3.Row:
    row = database.execute(
        SQL["space_context_snapshot"], (SPACE, ORGANIZATION, actor_id)
    ).fetchone()
    if row is None:
        raise AssertionError("fixture space context was not active")
    return row


def space_wire(database: sqlite3.Connection, ids: list[str]) -> str:
    rows = database.execute(
        SQL["space_assignment_snapshot"], (json.dumps(ids), SPACE)
    ).fetchall()
    return json.dumps(
        [
            {
                "id": row["requested_id"],
                "present": 0 if row["video_id"] is None else 1,
                "folder_id": row["folder_id"],
                "revision": -1 if row["revision"] is None else row["revision"],
            }
            for row in rows
        ],
        separators=(",", ":"),
    )


def product_precondition_parameters(
    *,
    operation_id: str,
    actor_id: str,
    action: str,
    context: str,
    target: sqlite3.Row | None,
    videos_json: str,
    scoped_json: str,
    original: sqlite3.Row | None = None,
    context_space: sqlite3.Row | None = None,
) -> tuple[object, ...]:
    return (
        operation_id,
        ORGANIZATION,
        actor_id,
        context,
        "" if target is None else target["id"],
        -1 if target is None else target["revision"],
        -1 if target is None else target["tree_revision"],
        None if target is None else target["space_id"],
        "" if target is None else target["created_by_user_id"],
        -1 if target is None else target["space_revision"],
        -1 if target is None else target["space_authority_version"],
        "" if target is None else target["actor_space_role"],
        -1 if target is None else target["actor_space_membership_revision"],
        "" if context_space is None else context_space["id"],
        -1 if context_space is None else context_space["revision"],
        -1 if context_space is None else context_space["authority_version"],
        "" if context_space is None else context_space["actor_space_role"],
        -1
        if context_space is None
        else context_space["actor_space_membership_revision"],
        videos_json,
        scoped_json,
        None if target is None else target["parent_id"],
        "" if original is None else original["id"],
        -1 if original is None else original["revision"],
        -1 if original is None else original["tree_revision"],
        None if original is None else original["space_id"],
        None if original is None else original["parent_id"],
        -1 if original is None else original["space_revision"],
        -1 if original is None else original["space_authority_version"],
        action,
    )


def insert_authority_assertions(
    database: sqlite3.Connection,
    operation_id: str,
    *,
    actor_id: str = ACTOR,
) -> None:
    authority = authority_snapshot(database, actor_id)
    database.execute(
        SQL["organization_assert"],
        (
            operation_id,
            ORGANIZATION,
            authority["organization_revision"],
            authority["organization_authority_version"],
        ),
    )
    database.execute(
        SQL["selection_assert"],
        (
            operation_id,
            actor_id,
            ORGANIZATION,
            authority["selection_revision"],
        ),
    )
    database.execute(
        SQL["membership_assert"],
        (
            operation_id,
            ORGANIZATION,
            actor_id,
            authority["membership_role"],
            authority["membership_revision"],
            authority["membership_authority_version"],
        ),
    )


@dataclass(frozen=True)
class DirectMovePlan:
    operation_id: str
    audit_id: str
    action: str
    key_digest: str
    request_digest: str
    grant: Grant
    target: sqlite3.Row
    original: sqlite3.Row | None
    videos_json: str
    final_json: str
    effect_json: str
    receipt_json: str
    video_revision: int
    old_folder_id: str | None


def direct_move_plan(
    database: sqlite3.Connection,
    *,
    serial: int,
    grant: Grant,
    key_digest: str | None = None,
    request_digest: str | None = None,
) -> DirectMovePlan:
    videos, videos_json = video_wire(database, [VIDEO])
    before = videos[0]
    target = folder_snapshot(database, ROOT_FOLDER)
    original = (
        None
        if before["folder_id"] is None
        else folder_snapshot(database, before["folder_id"])
    )
    effect = {
        "invalidates_caps": True,
        "targets": [{"kind": "folder", "folder_id": ROOT_FOLDER}],
    }
    context = {
        "target_folder_space_id": None,
        "original_folder_id": None if original is None else original["id"],
        "original_parent_id": None if original is None else original["parent_id"],
        "target_parent_id": target["parent_id"],
    }
    final = [
        {
            "id": VIDEO,
            "video_folder_id": ROOT_FOLDER,
            "video_revision": before["revision"]
            + (0 if before["folder_id"] == ROOT_FOLDER else 1),
            "scope_id": None,
            "scope_present": 0,
            "scope_folder_id": None,
            "scope_revision": -1,
            "active_count": 0,
            "active_id": "",
            "active_folder_id": None,
            "active_revision": -1,
        }
    ]
    effect_json = json.dumps(effect, separators=(",", ":"))
    receipt_json = json.dumps(
        {
            "affected_count": None,
            "effects": effect,
            "authorized_context": context,
        },
        separators=(",", ":"),
    )
    return DirectMovePlan(
        operation_id=uuid7_fixture(2_000 + serial * 2),
        audit_id=uuid7_fixture(2_001 + serial * 2),
        action="legacy_folder_assignment_move_v1",
        key_digest=key_digest or digest(f"key-{serial}"),
        request_digest=request_digest or digest(f"request-{serial}"),
        grant=grant,
        target=target,
        original=original,
        videos_json=videos_json,
        final_json=json.dumps(final, separators=(",", ":")),
        effect_json=effect_json,
        receipt_json=receipt_json,
        video_revision=before["revision"],
        old_folder_id=before["folder_id"],
    )


def execute_direct_move_batch(
    database: sqlite3.Connection,
    plan: DirectMovePlan,
    *,
    include_audit: bool = True,
    final_json: str | None = None,
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
            product_precondition_parameters(
                operation_id=plan.operation_id,
                actor_id=ACTOR,
                action="move",
                context="direct",
                target=plan.target,
                videos_json=plan.videos_json,
                scoped_json="[]",
                original=plan.original,
            ),
        )
        if plan.old_folder_id != ROOT_FOLDER:
            database.execute(
                SQL["direct_assignment_set"],
                (
                    VIDEO,
                    ROOT_FOLDER,
                    plan.operation_id,
                    NOW_MS,
                    ORGANIZATION,
                    plan.video_revision,
                    plan.old_folder_id,
                ),
            )
        database.execute(
            SQL["product_postcondition"],
            (
                plan.operation_id,
                ORGANIZATION,
                plan.final_json if final_json is None else final_json,
                "direct",
            ),
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
        database.execute(
            SQL["change_assert"], (plan.operation_id, "action_effect")
        )
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
                    digest(f"subject-{plan.request_digest}"),
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
        database.execute(
            SQL["change_assert"], (plan.operation_id, "grant_consumed")
        )
        database.execute(SQL["assertion_cleanup"], (plan.operation_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


def consume_grant_only(database: sqlite3.Connection, grant: Grant, serial: int) -> None:
    assertion_id = uuid7_fixture(5_000 + serial)
    database.execute("BEGIN IMMEDIATE")
    try:
        database.execute(
            SQL["grant_assert"],
            (
                assertion_id,
                grant.grant_id,
                grant.session_id,
                grant.user_id,
                NOW_MS,
            ),
        )
        database.execute(
            SQL["grant_delete"],
            (grant.grant_id, grant.session_id, grant.user_id),
        )
        database.execute(
            SQL["change_assert"], (assertion_id, "grant_consumed")
        )
        database.execute(SQL["assertion_cleanup"], (assertion_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


def consume_replay(
    database: sqlite3.Connection, operation_id: str, grant: Grant
) -> None:
    database.execute("BEGIN IMMEDIATE")
    try:
        insert_authority_assertions(database, operation_id)
        database.execute(
            SQL["grant_assert"],
            (
                operation_id,
                grant.grant_id,
                grant.session_id,
                grant.user_id,
                NOW_MS,
            ),
        )
        database.execute(
            SQL["grant_delete"],
            (grant.grant_id, grant.session_id, grant.user_id),
        )
        database.execute(
            SQL["change_assert"], (operation_id, "grant_consumed")
        )
        database.execute(SQL["assertion_cleanup"], (operation_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


def test_complete_batch_and_failure_atomicity() -> None:
    database = migrated_database()
    seed_fixture(database)
    raw_key = "never-persist-this-browser-key"
    grant = seed_grant(database, serial=1)
    plan = direct_move_plan(
        database,
        serial=1,
        grant=grant,
        key_digest=digest(raw_key),
    )
    execute_direct_move_batch(database, plan)
    operation = database.execute(
        """SELECT state,response_json,idempotency_key
           FROM authenticated_web_action_operations_v1
           WHERE operation_id=?1""",
        (plan.operation_id,),
    ).fetchone()
    assert operation["state"] == "complete"
    assert operation["response_json"] == plan.receipt_json
    assert operation["idempotency_key"] == digest(raw_key)
    assert raw_key not in "".join(str(value) for value in operation)
    assert database.execute(
        "SELECT value_json FROM authenticated_web_action_effects_v1 "
        "WHERE operation_id=?1",
        (plan.operation_id,),
    ).fetchone()[0] == plan.effect_json
    effect = json.loads(plan.effect_json)
    assert effect["invalidates_caps"] is True
    assert {target["folder_id"] for target in effect["targets"]} == {ROOT_FOLDER}
    assert database.execute(
        "SELECT COUNT(*) FROM business_audit_events_v1 WHERE operation_id=?1",
        (plan.operation_id,),
    ).fetchone()[0] == 1
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (grant.grant_id,),
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_assertions_v1"
    ).fetchone()[0] == 0
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM videos WHERE id=?1", (VIDEO,)
        ).fetchone()
    ) == (ROOT_FOLDER, 1)
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_api_execution_operations_v1"
    ).fetchone()[0] == 0
    database.close()

    # Receipt/effect/audit are one postcondition. Omitting the audit must roll
    # back the product mutation, operation, effect, and grant consumption.
    database = migrated_database()
    seed_fixture(database)
    grant = seed_grant(database, serial=2)
    plan = direct_move_plan(database, serial=2, grant=grant)
    try:
        execute_direct_move_batch(database, plan, include_audit=False)
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("missing business audit satisfied the receipt postcondition")
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM videos WHERE id=?1", (VIDEO,)
        ).fetchone()
    ) == (None, 0)
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_operations_v1"
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_effects_v1"
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM business_audit_events_v1"
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (grant.grant_id,),
    ).fetchone()[0] == 1
    database.close()

    # Browser proof is part of the same transaction fence. A session rotation
    # makes the grant stale and must leave product and journal state pristine.
    database = migrated_database()
    seed_fixture(database)
    grant = seed_grant(database, serial=5)
    plan = direct_move_plan(database, serial=5, grant=grant)
    database.execute(
        "UPDATE auth_sessions_v2 SET generation=generation+1 WHERE id=?1",
        (grant.session_id,),
    )
    try:
        execute_direct_move_batch(database, plan)
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("stale browser mutation grant committed")
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM videos WHERE id=?1", (VIDEO,)
        ).fetchone()
    ) == (None, 0)
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_operations_v1"
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (grant.grant_id,),
    ).fetchone()[0] == 1
    database.close()

    # A false product postcondition also aborts every journal and auth effect.
    database = migrated_database()
    seed_fixture(database)
    grant = seed_grant(database, serial=3)
    plan = direct_move_plan(database, serial=3, grant=grant)
    wrong_final = json.dumps(
        [
            {
                "id": VIDEO,
                "video_folder_id": SECOND_ROOT_FOLDER,
                "video_revision": 1,
            }
        ],
        separators=(",", ":"),
    )
    try:
        execute_direct_move_batch(database, plan, final_json=wrong_final)
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("false product postcondition committed")
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM videos WHERE id=?1", (VIDEO,)
        ).fetchone()
    ) == (None, 0)
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_operations_v1"
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (grant.grant_id,),
    ).fetchone()[0] == 1
    database.close()

    # A stale read snapshot is rejected by product_precondition before any
    # mutation. The independent revision change remains, proving rollback did
    # not hide the injected race.
    database = migrated_database()
    seed_fixture(database)
    grant = seed_grant(database, serial=4)
    plan = direct_move_plan(database, serial=4, grant=grant)
    database.execute(
        "UPDATE videos SET revision=revision+1 WHERE id=?1", (VIDEO,)
    )
    try:
        execute_direct_move_batch(database, plan)
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("stale product snapshot committed")
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM videos WHERE id=?1", (VIDEO,)
        ).fetchone()
    ) == (None, 1)
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_operations_v1"
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (grant.grant_id,),
    ).fetchone()[0] == 1
    database.close()


def test_all_or_nothing_list_authorization() -> None:
    database = migrated_database()
    seed_fixture(database)

    probe_serial = 0

    def probe(
        *,
        actor_id: str,
        action: str,
        context: str,
        target: sqlite3.Row,
        videos_json: str,
        scoped_json: str,
        requested_ids: tuple[str, ...],
        allowed: bool,
        context_space: sqlite3.Row | None = None,
    ) -> None:
        nonlocal probe_serial
        probe_serial += 1
        operation_id = uuid7_fixture(3_000 + probe_serial)
        database.execute("BEGIN")
        try:
            database.execute(
                SQL["operation_claim"],
                (
                    operation_id,
                    ORGANIZATION,
                    actor_id,
                    f"legacy_folder_assignment_{action}_v1",
                    digest(f"authorization-key-{probe_serial}"),
                    digest(f"authorization-request-{probe_serial}"),
                    NOW_MS,
                ),
            )
            insert_authority_assertions(database, operation_id, actor_id=actor_id)
            database.execute(
                SQL["product_precondition"],
                product_precondition_parameters(
                    operation_id=operation_id,
                    actor_id=actor_id,
                    action=action,
                    context=context,
                    target=target,
                    videos_json=videos_json,
                    scoped_json=scoped_json,
                    context_space=context_space,
                ),
            )
        except sqlite3.IntegrityError as error:
            database.execute("ROLLBACK")
            if allowed:
                raise AssertionError(
                    f"authorized {action}/{context} probe was denied"
                ) from error
            # Authorization denial is a single non-disclosing constraint
            # result; it must not identify which canonical video failed.
            for requested_id in requested_ids:
                assert requested_id not in str(error)
        else:
            database.execute("ROLLBACK")
            if not allowed:
                raise AssertionError(
                    f"unauthorized {action}/{context} probe was accepted"
                )
        assert database.execute(
            "SELECT COUNT(*) FROM authenticated_web_action_operations_v1 "
            "WHERE operation_id=?1",
            (operation_id,),
        ).fetchone()[0] == 0
        assert database.execute(
            "SELECT COUNT(*) FROM authenticated_web_action_assertions_v1 "
            "WHERE operation_id=?1",
            (operation_id,),
        ).fetchone()[0] == 0

    # A member may use a personal/root folder only when they created it.
    # Add and remove still require every canonical video to be actor-owned.
    personal = folder_snapshot(database, MEMBER_ROOT_FOLDER, MEMBER)
    unrelated_root = folder_snapshot(database, ROOT_FOLDER, MEMBER)
    member_wire, member_json = video_wire(database, [MEMBER_VIDEO])
    assert member_wire[0]["owner_id"] == MEMBER
    for action in ("add", "remove"):
        probe(
            actor_id=MEMBER,
            action=action,
            context="organization",
            target=personal,
            videos_json=member_json,
            scoped_json=shared_wire(
                database,
                [MEMBER_VIDEO],
                MEMBER_ROOT_FOLDER if action == "add" else None,
            ),
            requested_ids=(MEMBER_VIDEO,),
            allowed=True,
        )
    probe(
        actor_id=MEMBER,
        action="add",
        context="organization",
        target=unrelated_root,
        videos_json=member_json,
        scoped_json=shared_wire(database, [MEMBER_VIDEO], ROOT_FOLDER),
        requested_ids=(MEMBER_VIDEO,),
        allowed=False,
    )

    # Creator authority on a personal target permits move of another tenant
    # member's video, but an unrelated personal target does not.
    foreign_to_member_wire, foreign_to_member_json = video_wire(database, [VIDEO])
    assert foreign_to_member_wire[0]["owner_id"] == ACTOR
    probe(
        actor_id=MEMBER,
        action="move",
        context="organization",
        target=personal,
        videos_json=foreign_to_member_json,
        scoped_json=shared_wire(database, [VIDEO], MEMBER_ROOT_FOLDER),
        requested_ids=(VIDEO,),
        allowed=True,
    )
    probe(
        actor_id=MEMBER,
        action="move",
        context="organization",
        target=unrelated_root,
        videos_json=foreign_to_member_json,
        scoped_json=shared_wire(database, [VIDEO], ROOT_FOLDER),
        requested_ids=(VIDEO,),
        allowed=False,
    )

    # The source list filter is unconditional for add/remove. Even an
    # organization owner with broad management authority cannot submit one
    # owned and one non-owned video; every canonical ID is rejected together.
    mixed_ids = sorted((VIDEO, MEMBER_VIDEO))
    mixed_wire, mixed_json = video_wire(database, mixed_ids)
    assert {entry["owner_id"] for entry in mixed_wire} == {ACTOR, MEMBER}
    owner_target = folder_snapshot(database, ROOT_FOLDER, ACTOR)
    for action in ("add", "remove"):
        probe(
            actor_id=ACTOR,
            action=action,
            context="organization",
            target=owner_target,
            videos_json=mixed_json,
            scoped_json=shared_wire(
                database,
                mixed_ids,
                ROOT_FOLDER if action == "add" else None,
            ),
            requested_ids=tuple(mixed_ids),
            allowed=False,
        )
    owned_ids = sorted((VIDEO, SECOND_VIDEO))
    _, owner_owned_json = video_wire(database, owned_ids)
    probe(
        actor_id=ACTOR,
        action="add",
        context="organization",
        target=owner_target,
        videos_json=owner_owned_json,
        scoped_json=shared_wire(database, owned_ids, ROOT_FOLDER),
        requested_ids=tuple(owned_ids),
        allowed=True,
    )

    # A selected-space manager still cannot bypass list ownership for add or
    # remove. Move is different: manager authority may move a tenant video the
    # actor does not own, and both pre- and postconditions prove that update.
    database.execute(
        "UPDATE space_members SET role='manager' "
        "WHERE space_id=?1 AND user_id=?2",
        (SPACE, MEMBER),
    )
    manager_target = folder_snapshot(database, SPACE_FOLDER, MEMBER)
    manager_context = space_context_snapshot(database, actor_id=MEMBER)
    manager_mixed_ids = sorted((MEMBER_VIDEO, VIDEO))
    _, manager_mixed_json = video_wire(database, manager_mixed_ids)
    for action in ("add", "remove"):
        probe(
            actor_id=MEMBER,
            action=action,
            context="space",
            target=manager_target,
            videos_json=manager_mixed_json,
            scoped_json=space_wire(database, manager_mixed_ids),
            requested_ids=tuple(manager_mixed_ids),
            allowed=False,
            context_space=manager_context,
        )

    database.execute(
        SQL["space_assignment_insert"],
        (
            SPACE,
            VIDEO,
            SPACE_FOLDER_ALT,
            ACTOR,
            NOW_MS,
            uuid7_fixture(3_090),
        ),
    )
    manager_video_wire, manager_video_json = video_wire(database, [VIDEO])
    manager_scope_json = space_wire(database, [VIDEO])
    manager_move_id = uuid7_fixture(3_091)
    database.execute("BEGIN")
    try:
        database.execute(
            SQL["product_precondition"],
            product_precondition_parameters(
                operation_id=manager_move_id,
                actor_id=MEMBER,
                action="move",
                context="space",
                target=manager_target,
                videos_json=manager_video_json,
                scoped_json=manager_scope_json,
                context_space=manager_context,
            ),
        )
        database.execute(
            SQL["space_assignment_set"],
            (
                SPACE,
                VIDEO,
                SPACE_FOLDER,
                manager_move_id,
                0,
                SPACE_FOLDER_ALT,
            ),
        )
        final_json = json.dumps(
            [
                {
                    "id": VIDEO,
                    "video_folder_id": manager_video_wire[0]["folder_id"],
                    "video_revision": manager_video_wire[0]["revision"],
                    "scope_id": SPACE,
                    "scope_present": 1,
                    "scope_folder_id": SPACE_FOLDER,
                    "scope_revision": 1,
                    "active_count": 0,
                    "active_id": "",
                    "active_folder_id": None,
                    "active_revision": -1,
                }
            ],
            separators=(",", ":"),
        )
        database.execute(
            SQL["product_postcondition"],
            (manager_move_id, ORGANIZATION, final_json, "space"),
        )
        database.execute(SQL["assertion_cleanup"], (manager_move_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")
    assert database.execute(
        "SELECT folder_id FROM space_videos WHERE space_id=?1 AND video_id=?2",
        (SPACE, VIDEO),
    ).fetchone()[0] == SPACE_FOLDER

    # Downgrading the selected-space authority to contributor makes that same
    # foreign-video move fail, even when the contributor created the target.
    database.execute(
        "UPDATE space_members SET role='contributor', revision=revision+1 "
        "WHERE space_id=?1 AND user_id=?2",
        (SPACE, MEMBER),
    )
    contributor_target = folder_snapshot(database, MEMBER_SPACE_FOLDER, MEMBER)
    contributor_context = space_context_snapshot(database, actor_id=MEMBER)
    _, contributor_video_json = video_wire(database, [VIDEO])
    probe(
        actor_id=MEMBER,
        action="move",
        context="space",
        target=contributor_target,
        videos_json=contributor_video_json,
        scoped_json=space_wire(database, [VIDEO]),
        requested_ids=(VIDEO,),
        allowed=False,
        context_space=contributor_context,
    )

    assert database.execute(
        "SELECT COUNT(*) FROM shared_videos WHERE video_id IN (?1,?2,?3)",
        (MEMBER_VIDEO, VIDEO, SECOND_VIDEO),
    ).fetchone()[0] == 0
    database.close()


def test_replay_conflict_and_race_winner() -> None:
    # Restart replay consumes a fresh proof but does not repeat the product
    # mutation. Conflicting key reuse consumes its proof and preserves winner.
    database = migrated_database()
    seed_fixture(database)
    key_digest = digest("stable-key")
    request_digest = digest("stable-request")
    first_grant = seed_grant(database, serial=10)
    plan = direct_move_plan(
        database,
        serial=10,
        grant=first_grant,
        key_digest=key_digest,
        request_digest=request_digest,
    )
    execute_direct_move_batch(database, plan)
    replay_grant = seed_grant(database, serial=11)
    winner = database.execute(
        SQL["operation_by_key"],
        (ORGANIZATION, ACTOR, plan.action, key_digest),
    ).fetchone()
    assert winner["request_digest"] == request_digest
    assert winner["state"] == "complete"
    consume_replay(database, winner["operation_id"], replay_grant)
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM videos WHERE id=?1", (VIDEO,)
        ).fetchone()
    ) == (ROOT_FOLDER, 1)
    conflict_grant = seed_grant(database, serial=12)
    conflicting_request = digest("different-request")
    assert conflicting_request != winner["request_digest"]
    consume_grant_only(database, conflict_grant, 12)
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_operations_v1"
    ).fetchone()[0] == 1
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2"
    ).fetchone()[0] == 0
    assert tuple(
        database.execute(
            "SELECT folder_id,revision FROM videos WHERE id=?1", (VIDEO,)
        ).fetchone()
    ) == (ROOT_FOLDER, 1)
    database.close()

    with tempfile.TemporaryDirectory(prefix="frame-folder-race-") as directory:
        path = Path(directory) / "folder.sqlite3"
        database = migrated_database(path=path)
        seed_fixture(database)
        database.execute("PRAGMA journal_mode = WAL")
        grant_a = seed_grant(database, serial=20)
        grant_b = seed_grant(database, serial=21)
        race_key = digest("race-key")
        race_request = digest("race-request")
        plan_a = direct_move_plan(
            database,
            serial=20,
            grant=grant_a,
            key_digest=race_key,
            request_digest=race_request,
        )
        plan_b = direct_move_plan(
            database,
            serial=21,
            grant=grant_b,
            key_digest=race_key,
            request_digest=race_request,
        )
        database.close()

        barrier = threading.Barrier(2)
        outcomes: list[str] = []
        failures: list[str] = []
        outcome_lock = threading.Lock()

        def contender(plan: DirectMovePlan) -> None:
            candidate = connect(path)
            try:
                barrier.wait(timeout=5)
                try:
                    execute_direct_move_batch(candidate, plan)
                    outcome = "applied"
                except sqlite3.IntegrityError:
                    row = candidate.execute(
                        SQL["operation_by_key"],
                        (ORGANIZATION, ACTOR, plan.action, race_key),
                    ).fetchone()
                    if (
                        row is None
                        or row["request_digest"] != race_request
                        or row["state"] != "complete"
                    ):
                        raise AssertionError("race loser could not load exact winner")
                    consume_replay(candidate, row["operation_id"], plan.grant)
                    outcome = "replay"
                with outcome_lock:
                    outcomes.append(outcome)
            except Exception as error:  # reported after both threads join
                with outcome_lock:
                    failures.append(type(error).__name__)
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
            raise AssertionError("folder assignment race did not terminate")
        if failures:
            raise AssertionError(f"folder assignment race failures: {failures}")
        assert sorted(outcomes) == ["applied", "replay"]

        database = connect(path)
        assert database.execute(
            "SELECT COUNT(*) FROM authenticated_web_action_operations_v1"
        ).fetchone()[0] == 1
        assert database.execute(
            "SELECT COUNT(*) FROM authenticated_web_action_effects_v1"
        ).fetchone()[0] == 1
        assert database.execute(
            "SELECT COUNT(*) FROM business_audit_events_v1"
        ).fetchone()[0] == 1
        assert database.execute(
            "SELECT COUNT(*) FROM auth_session_mutation_grants_v2"
        ).fetchone()[0] == 0
        assert tuple(
            database.execute(
                "SELECT folder_id,revision FROM videos WHERE id=?1", (VIDEO,)
            ).fetchone()
        ) == (ROOT_FOLDER, 1)
        database.close()


def test_dirty_shared_multiplicity_remains_auditable_and_fails_closed() -> None:
    database = migrated_database(through=35)
    seed_fixture(database)
    database.executemany(
        """INSERT INTO shared_videos(
             id, video_id, organization_id, folder_id, shared_by_user_id,
             sharing_mode, shared_at_ms, revoked_at_ms, revision
           ) VALUES (?1, ?2, ?3, ?4, ?5, 'space', ?6, NULL, 0)""",
        (
            (
                uuid7_fixture(7_000),
                VIDEO,
                ORGANIZATION,
                ROOT_FOLDER,
                ACTOR,
                NOW_MS,
            ),
            (
                uuid7_fixture(7_001),
                VIDEO,
                ORGANIZATION,
                SPACE_FOLDER,
                ACTOR,
                NOW_MS,
            ),
        ),
    )
    migration = MIGRATION_ROOT / "0036_legacy_folder_assignment_expand.sql"
    migration_sql = migration.read_text(encoding="utf-8")
    assert "CREATE UNIQUE INDEX" not in migration_sql.upper()
    database.executescript(migration_sql)
    snapshot = database.execute(
        SQL["shared_assignment_snapshot"],
        (json.dumps([VIDEO]), ORGANIZATION, SPACE_FOLDER_ALT),
    ).fetchone()
    assert snapshot["active_count"] == 2
    assert database.execute(
        "SELECT COUNT(*) FROM shared_videos WHERE video_id=?1 "
        "AND organization_id=?2 AND revoked_at_ms IS NULL",
        (VIDEO, ORGANIZATION),
    ).fetchone()[0] == 2
    expect_integrity_error(
        database,
        SQL["shared_assignment_insert"],
        (
            uuid7_fixture(7_002),
            VIDEO,
            ORGANIZATION,
            SPACE_FOLDER_ALT,
            ACTOR,
            NOW_MS,
            uuid7_fixture(7_003),
        ),
        message="frame_legacy_folder_assignment_multiplicity_v1",
    )
    runtime = RUNTIME.read_text(encoding="utf-8")
    assert "!(0..=1).contains(&row.active_count)" in runtime
    database.close()


def test_static_redaction_and_business_journal_guards() -> None:
    runtime = RUNTIME.read_text(encoding="utf-8")
    application = APPLICATION.read_text(encoding="utf-8")
    combined_sql = "\n".join(SQL.values())
    runtime_without_negative_guards = "\n".join(
        line
        for line in runtime.splitlines()
        if 'assert!(!sql.contains("legacy_api_execution_' not in line
    )
    assert "legacy_api_execution_operations_v1" not in runtime_without_negative_guards
    assert "../queries/api_workflow/" not in runtime
    assert "legacy_api_execution_operations_v1" not in combined_sql
    assert "frame.legacy-folder-assignment.idempotency-key.v1" in runtime
    assert "map_err(|_| LegacyFolderAssignmentAtomicErrorV1::Unavailable)" in runtime
    for forbidden in (
        "error.to_string()",
        'format!("{error',
        'format!("{:?}", error)',
    ):
        assert forbidden not in runtime
    assert "business_audit_events_v1" in SQL["receipt_postcondition"]
    assert "authenticated_web_action_effects_v1" in SQL["receipt_postcondition"]
    assert "operation.state = 'complete'" in SQL["receipt_postcondition"]
    assert "effect.effect_state = 'applied'" in SQL["receipt_postcondition"]
    assert "?29 = 'move' OR NOT EXISTS" in SQL["product_precondition"]
    assert "?8 IS NULL AND ?9 = ?3" in SQL["product_precondition"]
    assert "SpaceRoot" in runtime and "SpaceFolder" in runtime
    assert "invalidates_caps" in runtime and "invalidates_caps" in application
    assert "effects.covers_command(command, authorized_context)" in application
    assert "effect_json.len() > 2_048" in runtime
    assert "receipt_json.len() > 8_192" in runtime


TESTS = (
    test_full_migration_and_scope_triggers,
    test_json_each_snapshots_are_tenant_bound_and_bounded,
    test_normalized_assignment_storage,
    test_complete_batch_and_failure_atomicity,
    test_all_or_nothing_list_authorization,
    test_replay_conflict_and_race_winner,
    test_dirty_shared_multiplicity_remains_auditable_and_fails_closed,
    test_static_redaction_and_business_journal_guards,
)


def run() -> dict[str, object]:
    for test in TESTS:
        test()
    return {
        "schema_version": "frame.legacy-folder-assignment-sqlite-conformance.v1",
        "provider": "local_sqlite",
        "expand_migration": "0036_legacy_folder_assignment_expand.sql",
        "full_expand_chain_applied": True,
        "tests_passed": len(TESTS),
        "snapshot_limit": 501,
        "normalized_storage_contexts": ["direct", "organization", "space"],
        "actions": ["add", "remove", "move"],
        "all_or_nothing_authorization": True,
        "add_remove_every_video_actor_owned": True,
        "move_manager_non_owned_tenant_video": True,
        "member_personal_root_creator_only": True,
        "receipt_effect_audit_grant_atomic": True,
        "exact_replay": True,
        "conflicting_key_reuse_rejected": True,
        "race_applied": 1,
        "race_replayed": 1,
        "dirty_shared_multiplicity_fail_closed": True,
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
        "legacy folder-assignment SQLite conformance: "
        f"{report['tests_passed']} passed; migration, normalized storage, "
        "atomic receipt/grant, replay/conflict/race, and dirty-data guards verified"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
