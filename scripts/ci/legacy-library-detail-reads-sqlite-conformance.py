#!/usr/bin/env python3
"""Provider-free D1/SQLite proof for Cap library-detail read semantics."""

from __future__ import annotations

import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_library_detail_reads"

ACTOR = "00000000-0000-4000-8000-000000000001"
OTHER = "00000000-0000-4000-8000-000000000002"
ORG = "10000000-0000-4000-8000-000000000001"
OTHER_ORG = "10000000-0000-4000-8000-000000000002"
SPACE = "20000000-0000-4000-8000-000000000001"
PRIVATE_SPACE = "20000000-0000-4000-8000-000000000002"
ORG_FOLDER = "30000000-0000-4000-8000-000000000001"
SPACE_FOLDER = "30000000-0000-4000-8000-000000000002"
PREFIX_VIDEO = "40000000-0000-4000-8000-000000000001"
CONTAINS_VIDEO = "40000000-0000-4000-8000-000000000002"
FOLDER_VIDEO = "40000000-0000-4000-8000-000000000003"
SHARED_VIDEO = "40000000-0000-4000-8000-000000000004"
SPACE_VIDEO = "40000000-0000-4000-8000-000000000005"
HIDDEN_VIDEO = "40000000-0000-4000-8000-000000000006"
CROSS_TENANT_VIDEO = "40000000-0000-4000-8000-000000000007"
ESCAPED_VIDEO = "40000000-0000-4000-8000-000000000008"
LEGACY_ORG = "0123456789abcde"
LEGACY_OTHER_ORG = "1123456789abcde"
LEGACY_SPACE = "2123456789abcde"
LEGACY_PRIVATE_SPACE = "3123456789abcde"
LEGACY_ACTOR = "4123456789abcde"
LEGACY_OTHER = "5123456789abcde"
LEGACY_VIDEOS = {
    PREFIX_VIDEO: "6123456789abcde",
    CONTAINS_VIDEO: "7123456789abcde",
    FOLDER_VIDEO: "8123456789abcde",
    SHARED_VIDEO: "9123456789abcde",
    SPACE_VIDEO: "a123456789abcde",
    HIDDEN_VIDEO: "b123456789abcde",
    CROSS_TENANT_VIDEO: "c123456789abcde",
    ESCAPED_VIDEO: "d123456789abcde",
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
    for user_id, email, name in [
        (ACTOR, "actor@example.test", "Actor"),
        (OTHER, "other@example.test", None),
    ]:
        connection.execute(
            """INSERT INTO users(
                 id,email,display_name,created_at_ms,updated_at_ms
               ) VALUES(?,?,?,?,?)""",
            (user_id, email, name, 1, 1),
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
    for organization_id, legacy_id in [
        (ORG, LEGACY_ORG),
        (OTHER_ORG, LEGACY_OTHER_ORG),
    ]:
        connection.execute(
            """INSERT INTO legacy_user_account_organization_ids_v1(
                 organization_id,legacy_organization_id,recorded_at_ms,last_operation_id
               ) VALUES(?,?,1,?)""",
            (organization_id, legacy_id, "50000000-0000-4000-8000-000000000001"),
        )
    for mapped_id, legacy_id in [(ACTOR, LEGACY_ACTOR), (OTHER, LEGACY_OTHER)]:
        connection.execute(
            """INSERT INTO legacy_collaboration_user_aliases_v1(
                 legacy_user_id,mapped_user_id,provenance,created_at_ms,refreshed_at_ms
               ) VALUES(?,?,'cap_backfill',1,1)""",
            (legacy_id, mapped_id),
        )
    for space_id, legacy_id, creator in [
        (SPACE, LEGACY_SPACE, ACTOR),
        (PRIVATE_SPACE, LEGACY_PRIVATE_SPACE, OTHER),
    ]:
        connection.execute(
            """INSERT INTO spaces(
                 id,organization_id,created_by_user_id,name,is_public,created_at_ms,updated_at_ms
               ) VALUES(?,?,?,'Space',0,1,1)""",
            (space_id, ORG, creator),
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
             created_at_ms,updated_at_ms,legacy_folder_id,legacy_name,
             legacy_color,legacy_scope_kind,legacy_scope_id
           ) VALUES(?,?,NULL,NULL,?,'retained placeholder',1,1,?,?,?,
                    'organization',?)""",
        (ORG_FOLDER, ORG, ACTOR, "e123456789abcde", "", "red", ORG),
    )
    connection.execute(
        """INSERT INTO folders(
             id,organization_id,space_id,parent_id,created_by_user_id,name,
             created_at_ms,updated_at_ms,legacy_folder_id,legacy_name,
             legacy_color,legacy_scope_kind,legacy_scope_id
           ) VALUES(?,?,?,NULL,?,'Space folder',1,1,?,?,
                    'blue','space',?)""",
        (SPACE_FOLDER, ORG, SPACE, ACTOR, "f123456789abcde", None, SPACE),
    )

    videos = [
        (PREFIX_VIDEO, ACTOR, ORG, "Demo old", 1_700_000_000_000,
         '{"customCreatedAt":"2024-01-02T03:04:05.006Z","nested":true}', 0, 1.25),
        (CONTAINS_VIDEO, ACTOR, ORG, "A demo new", 1_700_000_000_001,
         '{"customCreatedAt":"2026-01-02T03:04:05Z"}', 1, 2.5),
        (FOLDER_VIDEO, ACTOR, ORG, "Folder demo", 1_700_000_000_002,
         '{"customCreatedAt":"2025-01-02T03:04:05.123456Z"}', 0, None),
        (SHARED_VIDEO, OTHER, ORG, "Shared demo", 1_700_000_000_003, None, 0, None),
        (SPACE_VIDEO, OTHER, ORG, "Space demo", 1_700_000_000_004, None, 0, None),
        (HIDDEN_VIDEO, OTHER, ORG, "Hidden demo", 1_700_000_000_005, None, 0, None),
        (CROSS_TENANT_VIDEO, ACTOR, OTHER_ORG, "Cross demo", 1_700_000_000_006,
         None, 0, None),
        (ESCAPED_VIDEO, ACTOR, ORG, "100% _ ready", 1_700_000_000_007, None, 0, None),
    ]
    for video_id, owner_id, organization_id, title, created_at, metadata, screenshot, duration in videos:
        connection.execute(
            """INSERT INTO videos(
                 id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id,
                 legacy_metadata_json,legacy_is_screenshot,legacy_duration_seconds
               ) VALUES(?,?,?,'ready',?,?,?, ?,?,?)""",
            (
                video_id,
                owner_id,
                title,
                created_at,
                created_at,
                organization_id,
                metadata,
                screenshot,
                duration,
            ),
        )
        connection.execute(
            """INSERT INTO legacy_collaboration_video_aliases_v1(
                 legacy_video_id,mapped_video_id,provenance,created_at_ms
               ) VALUES(?,?,'cap_backfill',1)""",
            (LEGACY_VIDEOS[video_id], video_id),
        )

    for placement_id, video_id, folder_id, sharing_mode in [
        ("60000000-0000-4000-8000-000000000001", FOLDER_VIDEO, ORG_FOLDER, "space"),
        ("60000000-0000-4000-8000-000000000002", SHARED_VIDEO, None, "organization"),
    ]:
        connection.execute(
            """INSERT INTO shared_videos(
                 id,video_id,organization_id,folder_id,shared_by_user_id,
                 sharing_mode,shared_at_ms
               ) VALUES(?,?,?,?,?,?,1)""",
            (placement_id, video_id, ORG, folder_id, ACTOR, sharing_mode),
        )
    connection.execute(
        """INSERT INTO space_videos(
             space_id,video_id,folder_id,added_by_user_id,added_at_ms
           ) VALUES(?,?,?,?,1)""",
        (SPACE, CONTAINS_VIDEO, SPACE_FOLDER, ACTOR),
    )
    connection.execute(
        """INSERT INTO space_videos(
             space_id,video_id,folder_id,added_by_user_id,added_at_ms
           ) VALUES(?,?,NULL,?,1)""",
        (SPACE, SPACE_VIDEO, ACTOR),
    )
    connection.execute(
        """INSERT INTO space_videos(
             space_id,video_id,folder_id,added_by_user_id,added_at_ms
           ) VALUES(?,?,NULL,?,1)""",
        (PRIVATE_SPACE, HIDDEN_VIDEO, OTHER),
    )

    for index, kind in enumerate(["text", "text", "emoji"], start=1):
        connection.execute(
            """INSERT INTO legacy_collaboration_comments_v1(
                 legacy_comment_id,mapped_comment_id,legacy_video_id,mapped_video_id,
                 author_user_id,legacy_author_id,comment_kind,content,notification_kind,
                 created_at_ms,updated_at_ms,source_action,last_operation_id
               ) VALUES(?,?,?,?,?,?,?,?,?,1,1,'legacy.collaboration.cap_backfill',?)""",
            (
                f"{index}23456789abcdeg",
                f"70000000-0000-4000-8000-{index:012d}",
                LEGACY_VIDEOS[PREFIX_VIDEO],
                PREFIX_VIDEO,
                ACTOR,
                LEGACY_ACTOR,
                kind,
                kind,
                "reaction" if kind == "emoji" else "comment",
                f"80000000-0000-4000-8000-{index:012d}",
            ),
        )
    for index, video_id in enumerate([PREFIX_VIDEO, CONTAINS_VIDEO], start=1):
        connection.execute(
            """INSERT INTO video_uploads(
                 id,organization_id,video_id,state,expected_bytes,received_bytes,
                 idempotency_key,source_object_key,source_version,content_type,
                 created_at_ms,updated_at_ms,event_fingerprint
               ) VALUES(?,?,?,'initiated',1,0,?,?,1,'video/mp4',1,1,
                        'daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9')""",
            (
                f"90000000-0000-4000-8000-{index:012d}",
                ORG,
                video_id,
                f"upload-{index}",
                f"source-{index}",
            ),
        )
    connection.commit()


def rows(
    connection: sqlite3.Connection, name: str, parameters: tuple[str, ...]
) -> list[sqlite3.Row]:
    return connection.execute(sql(name), parameters).fetchall()


def main() -> None:
    connection = database()
    seed(connection)

    principal = rows(connection, "principal_scope.sql", (ACTOR,))
    assert len(principal) == 1
    assert principal[0]["active_organization_id"] == ORG
    assert principal[0]["active_legacy_organization_id"] == LEGACY_ORG

    assert len(rows(
        connection,
        "scope_authority.sql",
        (ACTOR, ORG, LEGACY_ORG, LEGACY_ORG),
    )) == 1
    assert len(rows(
        connection,
        "scope_authority.sql",
        (ACTOR, ORG, LEGACY_ORG, LEGACY_SPACE),
    )) == 1
    assert rows(
        connection,
        "scope_authority.sql",
        (ACTOR, ORG, LEGACY_ORG, LEGACY_PRIVATE_SPACE),
    ) == []

    organization_rows = rows(
        connection, "get_user_videos_organization.sql", (ACTOR, ORG)
    )
    organization_ids = [row["legacy_video_id"] for row in organization_rows]
    assert set(organization_ids) == {
        LEGACY_VIDEOS[PREFIX_VIDEO],
        LEGACY_VIDEOS[CONTAINS_VIDEO],
        LEGACY_VIDEOS[FOLDER_VIDEO],
        LEGACY_VIDEOS[ESCAPED_VIDEO],
    }
    assert LEGACY_VIDEOS[CROSS_TENANT_VIDEO] not in organization_ids
    prefix = next(
        row for row in organization_rows if row["legacy_video_id"] == LEGACY_VIDEOS[PREFIX_VIDEO]
    )
    assert prefix["total_comments"] == 2
    assert prefix["total_reactions"] == 1
    assert prefix["has_active_upload"] == 1
    assert prefix["metadata_json"].endswith('"nested":true}')
    screenshot = next(
        row for row in organization_rows if row["legacy_video_id"] == LEGACY_VIDEOS[CONTAINS_VIDEO]
    )
    assert screenshot["has_active_upload"] == 0
    folder = next(
        row for row in organization_rows if row["legacy_video_id"] == LEGACY_VIDEOS[FOLDER_VIDEO]
    )
    assert folder["folder_name"] == ""
    assert folder["folder_color"] == "red"
    exact_microseconds = connection.execute(
        "SELECT legacy_effective_created_at_us FROM videos WHERE id=?", (FOLDER_VIDEO,)
    ).fetchone()[0]
    assert exact_microseconds % 1_000_000 == 123_456
    assert [row["effective_created_at_ms"] for row in organization_rows] == sorted(
        [row["effective_created_at_ms"] for row in organization_rows], reverse=True
    )

    space_rows = rows(
        connection, "get_user_videos_space.sql", (ACTOR, LEGACY_SPACE, ORG)
    )
    space_folder = next(
        row for row in space_rows if row["legacy_video_id"] == LEGACY_VIDEOS[CONTAINS_VIDEO]
    )
    assert space_folder["folder_name"] == "Space folder"
    assert space_folder["folder_color"] == "blue"
    assert all(
        row["folder_name"] is None
        for row in space_rows
        if row["legacy_video_id"] != LEGACY_VIDEOS[CONTAINS_VIDEO]
    )

    search_rows = rows(
        connection,
        "search_dashboard_videos.sql",
        (ACTOR, ORG, "%demo%", "demo%"),
    )
    search_ids = [row["legacy_video_id"] for row in search_rows]
    assert search_ids[0] == LEGACY_VIDEOS[PREFIX_VIDEO]
    assert set(search_ids) == {
        LEGACY_VIDEOS[PREFIX_VIDEO],
        LEGACY_VIDEOS[CONTAINS_VIDEO],
        LEGACY_VIDEOS[FOLDER_VIDEO],
        LEGACY_VIDEOS[SHARED_VIDEO],
        LEGACY_VIDEOS[SPACE_VIDEO],
    }
    assert LEGACY_VIDEOS[HIDDEN_VIDEO] not in search_ids
    assert LEGACY_VIDEOS[CROSS_TENANT_VIDEO] not in search_ids
    assert search_rows[0]["duration_seconds"] == 1.25
    assert next(
        row for row in search_rows if row["legacy_video_id"] == LEGACY_VIDEOS[SHARED_VIDEO]
    )["owner_name"] is None

    escaped = rows(
        connection,
        "search_dashboard_videos.sql",
        (ACTOR, ORG, "%100!% !_ ready%", "100!% !_ ready%"),
    )
    assert [row["legacy_video_id"] for row in escaped] == [LEGACY_VIDEOS[ESCAPED_VIDEO]]

    print(
        "Legacy library-detail SQLite conformance passed: exact effective-date and prefix "
        "ordering, metadata/duration/screenshot/folder/count/upload projection, LIKE "
        "escaping, source visibility, scope decoration, and cross-tenant non-disclosure."
    )


if __name__ == "__main__":
    main()
