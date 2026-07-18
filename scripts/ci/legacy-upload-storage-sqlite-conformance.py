#!/usr/bin/env python3
"""SQLite proof for Cap upload/storage RPC, action, share, and workflow parity."""

from __future__ import annotations

import hashlib
import json
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_upload_storage"
FIXTURE = ROOT / "fixtures/api-parity/v1/upload-storage.json"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}

NOW = 1_735_787_045_006
ACTOR = "00000000-0000-7000-8000-000000000001"
MEMBER = "00000000-0000-7000-8000-000000000002"
FOREIGN = "00000000-0000-7000-8000-000000000003"
ORG = "00000000-0000-7000-8000-000000000101"
SHARED_ORG = "00000000-0000-7000-8000-000000000102"
FOREIGN_ORG = "00000000-0000-7000-8000-000000000103"
STORAGE = "00000000-0000-7000-8000-000000000201"
SPACE = "00000000-0000-7000-8000-000000000301"
VIDEO = "00000000-0000-7000-8000-000000000401"
LEGACY_ACTOR = "0123456789abcde"
LEGACY_MEMBER = "0123456789abcdf"
LEGACY_FOREIGN = "0123456789abcdg"
LEGACY_ORG = "1123456789abcde"
LEGACY_SHARED_ORG = "1123456789abcdf"
LEGACY_FOREIGN_ORG = "1123456789abcdg"
LEGACY_SPACE = "2123456789abcde"
LEGACY_VIDEO = "3123456789abcde"
PREFIX = f"{LEGACY_ACTOR}/{LEGACY_VIDEO}/"


def uid(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:", isolation_level=None)
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        if int(migration.name.split("_", 1)[0]) <= 57:
            connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def seed(connection: sqlite3.Connection) -> None:
    connection.executemany(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
        [
            (ACTOR, "actor@example.test", "Owner", 1, 1),
            (MEMBER, "member@example.test", "Member", 1, 1),
            (FOREIGN, "foreign@example.test", "Foreign", 1, 1),
        ],
    )
    connection.executemany(
        """INSERT INTO legacy_collaboration_user_aliases_v1(
             legacy_user_id,mapped_user_id,image_url,provenance,created_at_ms,refreshed_at_ms
           ) VALUES(?,?,NULL,'native_generated',1,1)""",
        [(LEGACY_ACTOR, ACTOR), (LEGACY_MEMBER, MEMBER), (LEGACY_FOREIGN, FOREIGN)],
    )
    connection.executemany(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
           ) VALUES(?,?,?,'active','{}',1,1)""",
        [(ORG, ACTOR, "Owner Org"), (SHARED_ORG, MEMBER, "Shared Org"), (FOREIGN_ORG, FOREIGN, "Foreign Org")],
    )
    connection.executemany(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,'owner','active',1,1,1)""",
        [(ORG, ACTOR), (SHARED_ORG, MEMBER), (FOREIGN_ORG, FOREIGN)],
    )
    # The owner may select only organizations in which they are active.
    connection.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,'member','active',0,1,1)""",
        (SHARED_ORG, ACTOR),
    )
    connection.executemany(
        """INSERT INTO legacy_user_account_organization_ids_v1(
             organization_id,legacy_organization_id,recorded_at_ms,last_operation_id
           ) VALUES(?,?,1,?)""",
        [
            (ORG, LEGACY_ORG, uid(801)),
            (SHARED_ORG, LEGACY_SHARED_ORG, uid(802)),
            (FOREIGN_ORG, LEGACY_FOREIGN_ORG, uid(803)),
        ],
    )
    connection.execute("UPDATE users SET active_organization_id=?,default_organization_id=? WHERE id=?", (ORG, ORG, ACTOR))
    capabilities = json.dumps({"schema_version": 1, "single_put": 1, "multipart": 1}, separators=(",", ":"))
    connection.execute(
        """INSERT INTO storage_integrations(
             id,organization_id,owner_user_id,provider,state,capabilities_json,
             credential_ciphertext,created_at_ms,updated_at_ms,capabilities_checksum
           ) VALUES(?,?,?,'r2','active',?,'sealed',1,1,?)""",
        (STORAGE, ORG, ACTOR, capabilities, digest(capabilities)),
    )
    connection.execute(
        """INSERT INTO spaces(
             id,organization_id,created_by_user_id,name,is_primary,is_public,
             settings_json,created_at_ms,updated_at_ms
           ) VALUES(?,?,?,'Shared Space',0,0,'{}',1,1)""",
        (SPACE, SHARED_ORG, MEMBER),
    )
    connection.execute(
        "INSERT INTO legacy_library_space_aliases_v1(legacy_space_id,space_id,provenance,created_at_ms) VALUES(?,?,'native_generated',1)",
        (LEGACY_SPACE, SPACE),
    )


def create_video(connection: sqlite3.Connection) -> None:
    operation = uid(900)
    authority = connection.execute(SQL["create_authority"], (ACTOR, LEGACY_ORG, None)).fetchall()
    assert len(authority) == 1 and authority[0]["legacy_actor_id"] == LEGACY_ACTOR
    connection.execute("BEGIN")
    connection.execute(
        SQL["create_video_insert"],
        (VIDEO, ACTOR, "Cap Upload - 2 January 2025", PREFIX + "result.mp4", 12_500, NOW, ORG, None, "public", operation, 1, 12.5, 0),
    )
    connection.execute(SQL["create_alias_insert"], (LEGACY_VIDEO, VIDEO, NOW))
    connection.execute(SQL["create_progress_insert"], (VIDEO, NOW))
    connection.execute(
        SQL["operation_insert"],
        (operation, "cap-v1-dd270efc913f9af9", "create_upload", ACTOR, ORG, VIDEO, LEGACY_VIDEO, digest("create-key"), digest("create-request"), "complete", '{"id":"3123456789abcde"}', NOW, NOW),
    )
    connection.execute(
        SQL["capability_insert"],
        (operation, STORAGE, PREFIX + "result.mp4", "video/mp4", NOW + 3_600_000, NOW),
    )
    connection.execute("COMMIT")
    media = connection.execute("SELECT * FROM legacy_mobile_cap_media_v1 WHERE mapped_video_id=?", (VIDEO,)).fetchone()
    assert media["object_prefix"] == PREFIX and media["source_type"] == "webMP4"
    replay = connection.execute(SQL["operation_replay"], ("cap-v1-dd270efc913f9af9", ACTOR, digest("create-key"))).fetchone()
    assert replay["state"] == "complete"


def prove_rpc_and_stale_fence(connection: sqlite3.Connection) -> None:
    row = connection.execute(SQL["read_authority"], (None, LEGACY_VIDEO)).fetchone()
    assert row["legacy_public"] == 1 and row["explicit_view"] == 0
    assert row["started_at_ms"] == NOW
    password_hash = "A" * 64
    connection.execute(
        "UPDATE videos SET legacy_password_hash=? WHERE id=?", (password_hash, VIDEO)
    )
    candidates = connection.execute(SQL["password_candidates"], (VIDEO,)).fetchall()
    assert [(item["password_hash"], item["ordinal"]) for item in candidates] == [
        (password_hash, 0)
    ]
    connection.execute("UPDATE videos SET legacy_password_hash=NULL WHERE id=?", (VIDEO,))
    edit_document = '{"schema_version":1}'
    connection.execute(
        """INSERT INTO video_edits(
             id,video_id,document_version,edit_spec_json,created_by_user_id,
             created_at_ms,updated_at_ms,document_checksum,last_operation_id
           ) VALUES(?,?,1,?,?,?,?,?,?)""",
        (
            uid(905),
            VIDEO,
            edit_document,
            ACTOR,
            NOW + 5,
            NOW + 5,
            digest(edit_document),
            uid(906),
        ),
    )
    edit_source = connection.execute(
        "SELECT source_key FROM legacy_upload_storage_edit_sources_v1 WHERE mapped_video_id=?",
        (VIDEO,),
    ).fetchone()
    assert edit_source["source_key"] == PREFIX + "source/original.mp4"
    connection.execute("DELETE FROM video_edits WHERE video_id=?", (VIDEO,))
    without_edit = connection.execute(
        SQL["read_authority"], (ACTOR, LEGACY_VIDEO)
    ).fetchone()
    assert without_edit["edit_source_key"] is None
    connection.execute(SQL["progress_upsert"], (VIDEO, 4, 10, NOW + 10))
    connection.execute(SQL["progress_upsert"], (VIDEO, 9, 10, NOW + 9))
    progress = connection.execute("SELECT * FROM legacy_mobile_cap_uploads_v1 WHERE mapped_video_id=?", (VIDEO,)).fetchone()
    assert progress["uploaded"] == 4 and progress["updated_at_ms"] == NOW + 10
    connection.execute(SQL["progress_upsert"], (VIDEO, 10, 10, NOW + 11))
    progress = connection.execute("SELECT * FROM legacy_mobile_cap_uploads_v1 WHERE mapped_video_id=?", (VIDEO,)).fetchone()
    assert progress["uploaded"] == 10 and progress["started_at_ms"] == NOW


def prove_native_progress_insert_retains_started_at(connection: sqlite3.Connection) -> None:
    """0052's pre-column trigger must still create an encodable progress row."""
    mapped_video = uid(410)
    legacy_video = "3123456789abcdf"
    source_key = f"{LEGACY_ACTOR}/{legacy_video}/result.mp4"
    connection.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,source_object_key,playback_object_key,
             duration_ms,created_at_ms,updated_at_ms,organization_id,privacy,
             revision,last_operation_id,legacy_public,legacy_is_screenshot
           ) VALUES(?,?,'Native progress','uploading',?,NULL,NULL,?,?,?,'private',0,?,0,0)""",
        (mapped_video, ACTOR, source_key, NOW + 40, NOW + 40, ORG, uid(903)),
    )
    connection.execute(SQL["create_alias_insert"], (legacy_video, mapped_video, NOW + 40))
    connection.execute(
        """INSERT INTO video_uploads(
             id,organization_id,video_id,state,expected_bytes,received_bytes,
             idempotency_key,source_object_key,source_version,content_type,
             created_at_ms,updated_at_ms
           ) VALUES(?,?,?,'initiated',?,0,?,?,1,'video/mp4',?,?)""",
        (
            uid(904),
            ORG,
            mapped_video,
            10,
            "native-started-at-proof",
            source_key,
            NOW + 41,
            NOW + 42,
        ),
    )
    progress = connection.execute(
        "SELECT started_at_ms,updated_at_ms FROM legacy_mobile_cap_uploads_v1 WHERE mapped_video_id=?",
        (mapped_video,),
    ).fetchone()
    assert progress["started_at_ms"] == NOW + 41
    assert progress["updated_at_ms"] == NOW + 42


def prove_share_replacement_filters_membership(connection: sqlite3.Connection) -> None:
    operation = uid(901)
    selected = json.dumps([LEGACY_SHARED_ORG, LEGACY_FOREIGN_ORG, LEGACY_SPACE])
    connection.execute("BEGIN")
    connection.execute(SQL["share_org_delete"], (VIDEO,))
    connection.execute(SQL["share_org_insert"], (VIDEO, ACTOR, selected, NOW + 20, operation))
    connection.execute(SQL["share_space_delete"], (VIDEO,))
    connection.execute(SQL["share_space_insert"], (VIDEO, ACTOR, selected, NOW + 20, operation))
    connection.execute(SQL["share_public_update"], (VIDEO, 0, NOW + 20, operation))
    connection.execute(
        SQL["operation_insert"],
        (operation, "cap-v1-55d41a7742153f1b", "share_cap", ACTOR, ORG, VIDEO, LEGACY_VIDEO, digest("share-key"), digest("share-request"), "complete", '{"success":true}', NOW + 20, NOW + 20),
    )
    connection.execute("COMMIT")
    organizations = [row[0] for row in connection.execute("SELECT organization_id FROM legacy_upload_storage_organization_shares_v1 WHERE mapped_video_id=?", (VIDEO,))]
    spaces = [row[0] for row in connection.execute("SELECT space_id FROM legacy_upload_storage_space_shares_v1 WHERE mapped_video_id=?", (VIDEO,))]
    assert organizations == [SHARED_ORG] and spaces == [SPACE]
    assert connection.execute("SELECT legacy_public FROM videos WHERE id=?", (VIDEO,)).fetchone()[0] == 0
    member_read = connection.execute(
        SQL["read_authority"], (MEMBER, LEGACY_VIDEO)
    ).fetchone()
    assert member_read["explicit_view"] == 1 and member_read["can_download"] == 1


def prove_reconcile_and_delete_continuation(connection: sqlite3.Connection) -> None:
    edit_key = PREFIX + "source/original.mp4"
    connection.execute(
        "UPDATE legacy_mobile_cap_uploads_v1 SET raw_file_key=?,phase='processing',processing_progress=0,updated_at_ms=? WHERE mapped_video_id=?",
        (edit_key, NOW - 16 * 60_000, VIDEO),
    )
    changed = connection.execute(SQL["reconcile_delete"], (VIDEO, edit_key, NOW - 16 * 60_000, "processing", 0)).rowcount
    assert changed == 1
    connection.execute(SQL["create_progress_insert"], (VIDEO, NOW + 30))
    operation = uid(902)
    connection.execute("BEGIN")
    connection.execute(
        SQL["operation_insert"],
        (operation, "cap-v1-6ed7083eeb37e3f8", "delete_result", ACTOR, ORG, VIDEO, LEGACY_VIDEO, digest("delete-key"), digest("delete-request"), "storage_pending", None, NOW + 31, None),
    )
    connection.execute(SQL["delete_progress"], (VIDEO,))
    connection.execute(SQL["delete_intent_insert"], (operation, STORAGE, PREFIX + "result.mp4", NOW + 31))
    connection.execute("COMMIT")
    assert connection.execute("SELECT state FROM legacy_upload_storage_operations_v1 WHERE operation_id=?", (operation,)).fetchone()[0] == "storage_pending"
    connection.execute("BEGIN")
    connection.execute(SQL["delete_intent_complete"], (operation, NOW + 32))
    connection.execute(SQL["operation_complete"], (operation, '{"success":true}', NOW + 32))
    connection.execute("COMMIT")
    assert connection.execute("SELECT state FROM legacy_upload_storage_delete_intents_v1 WHERE operation_id=?", (operation,)).fetchone()[0] == "complete"
    try:
        connection.execute("UPDATE legacy_upload_storage_operations_v1 SET request_digest=? WHERE operation_id=?", (digest("tampered"), operation))
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_upload_storage_operation_transition_v1" in str(error)
    else:
        raise AssertionError("immutable operation accepted tampering")


def main() -> None:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    assert fixture["schema_version"] == "frame.legacy-upload-storage.v1"
    assert len(fixture["operations"]) == 9
    connection = database()
    seed(connection)
    create_video(connection)
    prove_rpc_and_stale_fence(connection)
    prove_native_progress_insert_retains_started_at(connection)
    prove_share_replacement_filters_membership(connection)
    prove_reconcile_and_delete_continuation(connection)
    assert connection.execute("PRAGMA foreign_key_check").fetchall() == []
    print("legacy upload/storage SQLite conformance passed")


if __name__ == "__main__":
    main()
