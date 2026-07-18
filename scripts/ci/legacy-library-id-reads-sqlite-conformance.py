#!/usr/bin/env python3
"""Provider-free D1/SQLite proof for legacy library membership ID reads."""

from __future__ import annotations

import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_library_id_reads"

ACTOR = "00000000-0000-4000-8000-000000000001"
OTHER = "00000000-0000-4000-8000-000000000002"
ORG = "10000000-0000-4000-8000-000000000001"
OTHER_ORG = "10000000-0000-4000-8000-000000000002"
SPACE = "20000000-0000-4000-8000-000000000001"
PRIVATE_SPACE = "20000000-0000-4000-8000-000000000002"
ORG_FOLDER = "30000000-0000-4000-8000-000000000001"
SPACE_FOLDER = "30000000-0000-4000-8000-000000000002"
ROOT_VIDEO = "40000000-0000-4000-8000-000000000001"
ORG_FOLDER_VIDEO = "40000000-0000-4000-8000-000000000002"
SPACE_ROOT_VIDEO = "40000000-0000-4000-8000-000000000003"
SPACE_FOLDER_VIDEO = "40000000-0000-4000-8000-000000000004"
UNMAPPED_VIDEO = "40000000-0000-4000-8000-000000000005"
LEGACY_ORG = "0123456789abcde"
LEGACY_OTHER_ORG = "1123456789abcde"
LEGACY_SPACE = "2123456789abcde"
LEGACY_PRIVATE_SPACE = "3123456789abcde"
LEGACY_ORG_FOLDER = "4123456789abcde"
LEGACY_SPACE_FOLDER = "5123456789abcde"
LEGACY_VIDEOS = {
    ROOT_VIDEO: "6123456789abcde",
    ORG_FOLDER_VIDEO: "7123456789abcde",
    SPACE_ROOT_VIDEO: "8123456789abcde",
    SPACE_FOLDER_VIDEO: "9123456789abcde",
}


def sql(name: str) -> str:
    return (QUERIES / name).read_text(encoding="utf-8")


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:")
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def seed(connection: sqlite3.Connection) -> None:
    for user_id, email in [(ACTOR, "actor@example.test"), (OTHER, "other@example.test")]:
        connection.execute(
            "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
            (user_id, email, email.split("@")[0], 1, 1),
        )
    for organization_id, owner_id, name in [
        (ORG, ACTOR, "Actor organization"),
        (OTHER_ORG, OTHER, "Other organization"),
    ]:
        connection.execute(
            """INSERT INTO organizations(
                 id,owner_id,name,status,created_at_ms,updated_at_ms
               ) VALUES(?,?,?,'active',1,1)""",
            (organization_id, owner_id, name),
        )
        connection.execute(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,created_at_ms,updated_at_ms
               ) VALUES(?,?,'owner','active',1,1)""",
            (organization_id, owner_id),
        )
    connection.execute("UPDATE users SET active_organization_id=? WHERE id=?", (ORG, ACTOR))
    connection.execute(
        "UPDATE users SET active_organization_id=? WHERE id=?", (OTHER_ORG, OTHER)
    )
    for organization_id, legacy_id in [(ORG, LEGACY_ORG), (OTHER_ORG, LEGACY_OTHER_ORG)]:
        connection.execute(
            """INSERT INTO legacy_user_account_organization_ids_v1(
                 organization_id,legacy_organization_id,recorded_at_ms,last_operation_id
               ) VALUES(?,?,1,?)""",
            (organization_id, legacy_id, "50000000-0000-4000-8000-000000000001"),
        )
    for space_id, legacy_id, public in [
        (SPACE, LEGACY_SPACE, 0),
        (PRIVATE_SPACE, LEGACY_PRIVATE_SPACE, 0),
    ]:
        connection.execute(
            """INSERT INTO spaces(
                 id,organization_id,created_by_user_id,name,is_public,created_at_ms,updated_at_ms
               ) VALUES(?,?,?,?,?,1,1)""",
            (space_id, ORG, ACTOR if space_id == SPACE else OTHER, "Space", public),
        )
        connection.execute(
            """INSERT INTO legacy_library_space_aliases_v1(
                 legacy_space_id,space_id,provenance,created_at_ms
               ) VALUES(?,?,'cap_backfill',1)""",
            (legacy_id, space_id),
        )
    connection.execute(
        """INSERT INTO space_members(
             space_id,user_id,role,created_at_ms,updated_at_ms,state
           ) VALUES(?,?,'viewer',1,1,'active')""",
        (SPACE, ACTOR),
    )
    connection.execute(
        """INSERT INTO folders(
             id,organization_id,space_id,parent_id,created_by_user_id,name,
             created_at_ms,updated_at_ms,legacy_folder_id,legacy_scope_kind,legacy_scope_id
           ) VALUES(?,?,NULL,NULL,?,'Org folder',1,1,?,'organization',?)""",
        (ORG_FOLDER, ORG, ACTOR, LEGACY_ORG_FOLDER, ORG),
    )
    connection.execute(
        """INSERT INTO folders(
             id,organization_id,space_id,parent_id,created_by_user_id,name,
             created_at_ms,updated_at_ms,legacy_folder_id,legacy_scope_kind,legacy_scope_id
           ) VALUES(?,?,?,NULL,?,'Space folder',1,1,?,'space',?)""",
        (SPACE_FOLDER, ORG, SPACE, ACTOR, LEGACY_SPACE_FOLDER, SPACE),
    )
    for video_id in [
        ROOT_VIDEO,
        ORG_FOLDER_VIDEO,
        SPACE_ROOT_VIDEO,
        SPACE_FOLDER_VIDEO,
        UNMAPPED_VIDEO,
    ]:
        connection.execute(
            """INSERT INTO videos(
                 id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id
               ) VALUES(?,?,'Video','ready',1,1,?)""",
            (video_id, ACTOR, ORG),
        )
    for mapped_video_id, legacy_video_id in LEGACY_VIDEOS.items():
        connection.execute(
            """INSERT INTO legacy_collaboration_video_aliases_v1(
                 legacy_video_id,mapped_video_id,provenance,created_at_ms
               ) VALUES(?,?,'cap_backfill',1)""",
            (legacy_video_id, mapped_video_id),
        )
    for placement_id, video_id, folder_id, sharing_mode in [
        ("60000000-0000-4000-8000-000000000001", ROOT_VIDEO, None, "organization"),
        ("60000000-0000-4000-8000-000000000002", ORG_FOLDER_VIDEO, ORG_FOLDER, "space"),
        ("60000000-0000-4000-8000-000000000003", UNMAPPED_VIDEO, None, "organization"),
    ]:
        connection.execute(
            """INSERT INTO shared_videos(
                 id,video_id,organization_id,folder_id,shared_by_user_id,
                 sharing_mode,shared_at_ms
               ) VALUES(?,?,?,?,?,?,1)""",
            (placement_id, video_id, ORG, folder_id, ACTOR, sharing_mode),
        )
    for space_id, video_id, folder_id in [
        (SPACE, SPACE_ROOT_VIDEO, None),
        (SPACE, SPACE_FOLDER_VIDEO, SPACE_FOLDER),
    ]:
        connection.execute(
            """INSERT INTO space_videos(
                 space_id,video_id,folder_id,added_by_user_id,added_at_ms
               ) VALUES(?,?,?,?,1)""",
            (space_id, video_id, folder_id, ACTOR),
        )
    connection.commit()


def rows(connection: sqlite3.Connection, name: str, parameters: tuple[str, ...]) -> list[sqlite3.Row]:
    return connection.execute(sql(name), parameters).fetchall()


def main() -> None:
    connection = database()
    seed(connection)

    principal = rows(connection, "principal_scope.sql", (ACTOR,))
    assert len(principal) == 1
    assert principal[0]["active_organization_id"] == ORG
    assert principal[0]["active_legacy_organization_id"] == LEGACY_ORG

    assert len(rows(connection, "organization_authority.sql", (ACTOR, LEGACY_ORG, ORG))) == 1
    assert rows(connection, "organization_authority.sql", (ACTOR, LEGACY_OTHER_ORG, ORG)) == []
    organization_ids = rows(connection, "organization_video_ids.sql", (LEGACY_ORG, ORG))
    assert {row["legacy_video_id"] for row in organization_ids} == {
        LEGACY_VIDEOS[ROOT_VIDEO],
        None,
    }

    assert len(rows(connection, "space_authority.sql", (ACTOR, LEGACY_SPACE, ORG))) == 1
    assert rows(connection, "space_authority.sql", (ACTOR, LEGACY_PRIVATE_SPACE, ORG)) == []
    assert [row["legacy_video_id"] for row in rows(
        connection, "space_video_ids.sql", (LEGACY_SPACE,)
    )] == [LEGACY_VIDEOS[SPACE_ROOT_VIDEO]]

    assert len(rows(
        connection,
        "folder_authority.sql",
        (ACTOR, ORG, LEGACY_ORG_FOLDER, LEGACY_ORG, LEGACY_ORG),
    )) == 1
    assert [row["legacy_video_id"] for row in rows(
        connection, "folder_video_ids_organization.sql", (LEGACY_ORG_FOLDER, ORG)
    )] == [LEGACY_VIDEOS[ORG_FOLDER_VIDEO]]
    assert len(rows(
        connection,
        "folder_authority.sql",
        (ACTOR, ORG, LEGACY_SPACE_FOLDER, LEGACY_SPACE, LEGACY_ORG),
    )) == 1
    assert [row["legacy_video_id"] for row in rows(
        connection,
        "folder_video_ids_space.sql",
        (LEGACY_SPACE_FOLDER, LEGACY_SPACE, ORG),
    )] == [LEGACY_VIDEOS[SPACE_FOLDER_VIDEO]]
    assert rows(
        connection,
        "folder_authority.sql",
        (ACTOR, ORG, LEGACY_SPACE_FOLDER, LEGACY_PRIVATE_SPACE, LEGACY_ORG),
    ) == []

    try:
        connection.execute(
            "UPDATE legacy_library_space_aliases_v1 SET legacy_space_id=? WHERE space_id=?",
            ("a123456789abcde", SPACE),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_library_alias_immutable_v1" in str(error)
    else:
        raise AssertionError("space alias update unexpectedly succeeded")

    print(
        "Legacy library-ID read SQLite conformance passed: active-tenant authority, "
        "organization/space/folder branches, alias corruption detection, cross-tenant "
        "non-disclosure, and immutable scope aliases."
    )


if __name__ == "__main__":
    main()
