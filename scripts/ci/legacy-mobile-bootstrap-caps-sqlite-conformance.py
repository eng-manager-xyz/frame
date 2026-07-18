#!/usr/bin/env python3
"""SQLite proof for Cap mobile bootstrap, cap reads, and delete ordering."""

from __future__ import annotations

import hashlib
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_mobile_bootstrap_caps"
FIXTURE = ROOT / "fixtures/api-parity/v1/mobile-bootstrap-caps.json"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}
NOW = 1_735_787_045_006
ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"


def uid(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def cap_id(number: int) -> str:
    output = []
    for _ in range(15):
        output.append(ALPHABET[number & 31])
        number >>= 5
    return "".join(reversed(output))


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:", isolation_level=None)
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def insert_user(
    connection: sqlite3.Connection, number: int, email: str, name: str
) -> tuple[str, str]:
    mapped_id = uid(number)
    legacy_id = cap_id(number)
    connection.execute(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
        (mapped_id, email, name, NOW - 10_000, NOW - 10_000),
    )
    connection.execute(
        """INSERT INTO legacy_collaboration_user_aliases_v1(
             legacy_user_id,mapped_user_id,image_url,provenance,created_at_ms,refreshed_at_ms
           ) VALUES(?,?,NULL,'native_generated',?,?)""",
        (legacy_id, mapped_id, NOW - 10_000, NOW - 10_000),
    )
    return mapped_id, legacy_id


def insert_organization(
    connection: sqlite3.Connection, number: int, owner_id: str
) -> tuple[str, str]:
    mapped_id = uid(100 + number)
    legacy_id = cap_id(100 + number)
    connection.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
           ) VALUES(?,?,'Mobile Org','active','{}',?,?)""",
        (mapped_id, owner_id, NOW - 9_000, NOW - 9_000),
    )
    connection.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,'owner','active',0,?,?)""",
        (mapped_id, owner_id, NOW - 9_000, NOW - 9_000),
    )
    connection.execute(
        """INSERT INTO legacy_user_account_organization_ids_v1(
             organization_id,legacy_organization_id,recorded_at_ms,last_operation_id
           ) VALUES(?,?,?,?)""",
        (mapped_id, legacy_id, NOW - 9_000, uid(8_000 + number)),
    )
    return mapped_id, legacy_id


def fixture_graph(connection: sqlite3.Connection) -> dict[str, str]:
    actor, legacy_actor = insert_user(
        connection, 1, "owner@example.test", "Mobile Owner"
    )
    foreign, _ = insert_user(connection, 2, "foreign@example.test", "Foreign User")
    organization, legacy_organization = insert_organization(connection, 1, actor)
    connection.execute(
        "UPDATE users SET active_organization_id=?, default_organization_id=? WHERE id=?",
        (organization, organization, actor),
    )
    folder = uid(300)
    legacy_folder = cap_id(300)
    connection.execute(
        """INSERT INTO folders(
             id,organization_id,space_id,parent_id,created_by_user_id,name,is_public,
             settings_json,created_at_ms,updated_at_ms,legacy_folder_id,legacy_name,
             legacy_color,legacy_scope_kind,legacy_scope_id
           ) VALUES(?,?,NULL,NULL,?,'Folder',0,'{}',?,?,?,'Folder','blue','personal',NULL)""",
        (folder, organization, actor, NOW - 8_000, NOW - 8_000, legacy_folder),
    )
    video = uid(400)
    legacy_video = cap_id(400)
    connection.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,source_object_key,playback_object_key,duration_ms,
             created_at_ms,updated_at_ms,organization_id,folder_id,privacy,metadata_json,
             legacy_public,legacy_metadata_json,legacy_is_screenshot,legacy_duration_seconds
           ) VALUES(?,?,'Exact Mobile Cap','ready',?,?,12500,?,?,?,?, 'private',NULL,
                    1,?,0,12.5)""",
        (
            video,
            actor,
            f"{legacy_actor}/{legacy_video}/result.mp4",
            f"{legacy_actor}/{legacy_video}/result.mp4",
            NOW - 7_000,
            NOW - 6_000,
            organization,
            folder,
            '{"summary":"Pinned summary","chapters":[{"title":"Intro","start":0}]}',
        ),
    )
    connection.execute(
        """INSERT INTO legacy_collaboration_video_aliases_v1(
             legacy_video_id,mapped_video_id,provenance,created_at_ms
           ) VALUES(?,?,'native_generated',?)""",
        (legacy_video, video, NOW - 7_000),
    )
    comment = cap_id(500)
    connection.execute(
        """INSERT INTO legacy_collaboration_comments_v1(
             legacy_comment_id,mapped_comment_id,legacy_video_id,mapped_video_id,
             author_user_id,legacy_author_id,comment_kind,content,source_timestamp,
             legacy_parent_comment_id,notification_kind,created_at_ms,updated_at_ms,
             source_action,last_operation_id
           ) VALUES(?,?,?,?,?,?,'text','hello',1.25,NULL,'comment',?,?,'legacy.collaboration.cap_backfill',?)""",
        (
            comment,
            uid(500),
            legacy_video,
            video,
            actor,
            legacy_actor,
            NOW - 5_000,
            NOW - 5_000,
            uid(8_500),
        ),
    )
    connection.execute(
        """INSERT INTO video_uploads(
             id,organization_id,video_id,state,expected_bytes,received_bytes,
             idempotency_key,source_object_key,source_version,content_type,
             created_at_ms,updated_at_ms,event_fingerprint
           ) VALUES(?,?,?,'initiated',100,0,'mobile-upload',?,1,'video/mp4',?,?,?)""",
        (
            uid(600),
            organization,
            video,
            f"{legacy_actor}/{legacy_video}/raw.webm",
            NOW - 4_000,
            NOW - 4_000,
            "daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9",
        ),
    )
    connection.execute(
        """UPDATE video_uploads
           SET state='uploading', received_bytes=40, event_sequence=1,
               event_fingerprint=?, updated_at_ms=?
           WHERE id=?""",
        (digest("mobile-upload-started"), NOW - 3_900, uid(600)),
    )
    return {
        "actor": actor,
        "legacy_actor": legacy_actor,
        "foreign": foreign,
        "organization": organization,
        "legacy_organization": legacy_organization,
        "folder": folder,
        "legacy_folder": legacy_folder,
        "video": video,
        "legacy_video": legacy_video,
    }


def prove_owner_scoped_list_detail_projection(connection: sqlite3.Connection) -> None:
    """Owner-scoped list/detail projection preserves active-tenant and wire fields."""
    graph = fixture_graph(connection)
    actor = connection.execute(SQL["actor_profile"], (graph["actor"],)).fetchone()
    assert actor["legacy_user_id"] == graph["legacy_actor"]
    assert actor["active_legacy_organization_id"] == graph["legacy_organization"]
    organizations = connection.execute(SQL["organizations"], (graph["actor"],)).fetchall()
    assert [(row["legacy_organization_id"], row["effective_role"]) for row in organizations] == [
        (graph["legacy_organization"], "owner")
    ]
    folders = connection.execute(
        SQL["root_folders"], (graph["actor"], graph["organization"])
    ).fetchall()
    assert len(folders) == 1
    assert folders[0]["legacy_folder_id"] == graph["legacy_folder"]
    assert folders[0]["video_count"] == 1
    count = connection.execute(
        SQL["caps_count"],
        (graph["actor"], graph["organization"], graph["legacy_folder"]),
    ).fetchone()["total"]
    rows = connection.execute(
        SQL["caps_rows"],
        (graph["actor"], graph["organization"], graph["legacy_folder"], 20, 0),
    ).fetchall()
    assert count == 1 and len(rows) == 1
    assert rows[0]["legacy_video_id"] == graph["legacy_video"]
    assert rows[0]["comment_count"] == 1 and rows[0]["reaction_count"] == 0
    assert rows[0]["upload_uploaded"] == 40.0
    detail = connection.execute(
        SQL["cap_row"], (graph["actor"], graph["legacy_video"])
    ).fetchone()
    assert detail["raw_file_key"].endswith("/raw.webm")
    comments = connection.execute(SQL["comments"], (graph["legacy_video"],)).fetchall()
    assert len(comments) == 1 and comments[0]["content"] == "hello"


def prove_tenant_non_disclosure(connection: sqlite3.Connection) -> None:
    """Tenant non-disclosure makes a foreign detail and delete target indistinguishable."""
    graph = fixture_graph(connection)
    assert (
        connection.execute(
            SQL["cap_row"], (graph["foreign"], graph["legacy_video"])
        ).fetchone()
        is None
    )
    assert (
        connection.execute(
            SQL["delete_snapshot"], (graph["foreign"], graph["legacy_video"])
        ).fetchone()
        is None
    )


def prove_upload_media_dual_write_triggers(connection: sqlite3.Connection) -> None:
    """Upload/media dual-write triggers retain exact source and progress projections."""
    graph = fixture_graph(connection)
    media = connection.execute(
        "SELECT * FROM legacy_mobile_cap_media_v1 WHERE mapped_video_id=?",
        (graph["video"],),
    ).fetchone()
    assert media["source_type"] == "webMP4"
    assert media["object_prefix"] == f'{graph["legacy_actor"]}/{graph["legacy_video"]}/'
    connection.execute(
        "UPDATE video_uploads SET received_bytes=80,updated_at_ms=? WHERE video_id=?",
        (NOW - 3_000, graph["video"]),
    )
    upload = connection.execute(
        "SELECT * FROM legacy_mobile_cap_uploads_v1 WHERE mapped_video_id=?",
        (graph["video"],),
    ).fetchone()
    assert upload["uploaded"] == 80.0 and upload["total"] == 100.0
    assert upload["phase"] == "uploading"


def prove_delete_d1_before_provider_cleanup(connection: sqlite3.Connection) -> None:
    """Delete commits D1 before provider cleanup and retains a guarded continuation."""
    graph = fixture_graph(connection)
    snapshot = connection.execute(
        SQL["delete_snapshot"], (graph["actor"], graph["legacy_video"])
    ).fetchone()
    operation_id = uid(900)
    audit_id = uid(901)
    connection.execute("BEGIN")
    connection.execute(
        SQL["delete_apply"],
        (graph["actor"], graph["legacy_video"], graph["video"], NOW),
    )
    connection.execute(
        SQL["delete_operation_insert"],
        (
            operation_id,
            graph["actor"],
            graph["video"],
            graph["legacy_video"],
            snapshot["object_prefix"],
            NOW,
        ),
    )
    connection.execute(
        SQL["delete_audit_insert"],
        (audit_id, operation_id, digest(graph["actor"]), digest(graph["legacy_video"]), NOW),
    )
    for kind in ("authority", "tombstone"):
        connection.execute(
            SQL["delete_assert"],
            (
                operation_id,
                kind,
                graph["video"],
                graph["actor"],
                graph["legacy_video"],
                NOW,
            ),
        )
    connection.execute("COMMIT")
    assert (
        connection.execute(
            SQL["cap_row"], (graph["actor"], graph["legacy_video"])
        ).fetchone()
        is None
    )
    pending = connection.execute(
        "SELECT state FROM legacy_mobile_cap_delete_operations_v1 WHERE operation_id=?",
        (operation_id,),
    ).fetchone()
    assert pending["state"] == "storage_pending"
    connection.execute("BEGIN")
    connection.execute(SQL["delete_complete"], (operation_id, NOW + 1))
    connection.execute(SQL["delete_cleanup_assert"], (operation_id, NOW + 1))
    connection.execute("COMMIT")
    assert connection.execute(
        "SELECT state FROM legacy_mobile_cap_delete_operations_v1 WHERE operation_id=?",
        (operation_id,),
    ).fetchone()["state"] == "complete"
    immutable_mutations = (
        (
            "UPDATE legacy_mobile_cap_delete_operations_v1 SET state='storage_pending',completed_at_ms=NULL WHERE operation_id=?",
            (operation_id,),
        ),
        (
            "UPDATE legacy_mobile_cap_delete_audit_v1 SET outcome=outcome WHERE audit_id=?",
            (audit_id,),
        ),
        (
            "UPDATE legacy_mobile_cap_delete_assertions_v1 SET actual_count=actual_count WHERE operation_id=?",
            (operation_id,),
        ),
        (
            "DELETE FROM legacy_mobile_cap_delete_assertions_v1 WHERE operation_id=?",
            (operation_id,),
        ),
    )
    for statement, bindings in immutable_mutations:
        try:
            connection.execute(statement, bindings)
        except sqlite3.IntegrityError as error:
            assert "frame_legacy_mobile_cap_delete_evidence_immutable_v1" in str(error)
        else:
            raise AssertionError("completed delete evidence became mutable")


def main() -> None:
    assert FIXTURE.exists()
    for proof in (
        prove_owner_scoped_list_detail_projection,
        prove_tenant_non_disclosure,
        prove_upload_media_dual_write_triggers,
        prove_delete_d1_before_provider_cleanup,
    ):
        connection = database()
        try:
            proof(connection)
        finally:
            connection.close()
    print("legacy mobile bootstrap/caps SQLite conformance: ok")


if __name__ == "__main__":
    main()
