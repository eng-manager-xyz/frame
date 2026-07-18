#!/usr/bin/env python3
"""SQLite proof for Effect-RPC video lifecycle D1/R2 orchestration."""

from __future__ import annotations

import hashlib
import json
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_video_lifecycle"
FIXTURE = ROOT / "fixtures/api-parity/v1/video-lifecycle.json"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}

NOW = 1_735_787_045_006
ACTOR = "00000000-0000-7000-8000-000000000001"
FOREIGN = "00000000-0000-7000-8000-000000000002"
ORG = "00000000-0000-7000-8000-000000000101"
FOREIGN_ORG = "00000000-0000-7000-8000-000000000102"
VIDEO = "00000000-0000-7000-8000-000000000201"
DUPLICATE = "00000000-0000-7000-8000-000000000202"
INSTANT = "00000000-0000-7000-8000-000000000203"
LEGACY_ACTOR = "0123456789abcde"
LEGACY_FOREIGN = "0123456789abcdf"
LEGACY_VIDEO = "1123456789abcde"
LEGACY_DUPLICATE = "1123456789abcdf"
LEGACY_INSTANT = "1123456789abcdg"
PREFIX = f"{LEGACY_ACTOR}/{LEGACY_VIDEO}/"
DUPLICATE_PREFIX = f"{LEGACY_ACTOR}/{LEGACY_DUPLICATE}/"
INSTANT_PREFIX = f"{LEGACY_ACTOR}/{LEGACY_INSTANT}/"


def uid(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:", isolation_level=None)
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        if int(migration.name.split("_", 1)[0]) <= 60:
            connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def seed(connection: sqlite3.Connection) -> None:
    connection.executemany(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
        [
            (ACTOR, "owner@example.test", "Owner", 1, 1),
            (FOREIGN, "foreign@example.test", "Foreign", 1, 1),
        ],
    )
    connection.executemany(
        """INSERT INTO legacy_collaboration_user_aliases_v1(
             legacy_user_id,mapped_user_id,image_url,provenance,created_at_ms,refreshed_at_ms
           ) VALUES(?,?,NULL,'native_generated',1,1)""",
        [(LEGACY_ACTOR, ACTOR), (LEGACY_FOREIGN, FOREIGN)],
    )
    connection.executemany(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
           ) VALUES(?,?,?,'active','{}',1,1)""",
        [(ORG, ACTOR, "Lifecycle"), (FOREIGN_ORG, FOREIGN, "Foreign")],
    )
    connection.executemany(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,'owner','active',1,1,1)""",
        [(ORG, ACTOR), (FOREIGN_ORG, FOREIGN)],
    )
    connection.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,source_object_key,playback_object_key,duration_ms,
             created_at_ms,updated_at_ms,organization_id,privacy,metadata_json,
             legacy_public,legacy_password_hash,legacy_settings_json,legacy_metadata_json,
             legacy_is_screenshot,legacy_duration_seconds,legacy_storage_width,
             legacy_storage_height,legacy_storage_fps
           ) VALUES(?,?,'Source','ready',?,NULL,12500,1,1,?,'public',NULL,1,?,
                    '{"source":"settings"}','{"source":"metadata"}',0,12.5,1920,1080,60)""",
        (VIDEO, ACTOR, PREFIX + "result.mp4", ORG, "a" * 64),
    )
    connection.execute(
        """INSERT INTO legacy_collaboration_video_aliases_v1(
             legacy_video_id,mapped_video_id,provenance,created_at_ms
           ) VALUES(?,?,'native_generated',1)""",
        (LEGACY_VIDEO, VIDEO),
    )


def prove_authority_and_og_non_disclosure(connection: sqlite3.Connection) -> None:
    owner = connection.execute(SQL["video_owner_snapshot"], (ACTOR, LEGACY_VIDEO)).fetchall()
    assert len(owner) == 1 and owner[0]["object_prefix"] == PREFIX
    assert (
        connection.execute(SQL["video_owner_snapshot"], (FOREIGN, LEGACY_VIDEO)).fetchone()
        is None
    ), "tenant isolation lost"
    og = connection.execute(SQL["og_snapshot"], (LEGACY_VIDEO,)).fetchall()
    assert len(og) == 1 and og[0]["legacy_public"] == 1
    connection.execute("UPDATE videos SET legacy_public=0,privacy='private' WHERE id=?", (VIDEO,))
    private = connection.execute(SQL["og_snapshot"], (LEGACY_VIDEO,)).fetchone()
    assert private is not None and private["legacy_public"] == 0
    connection.execute("UPDATE videos SET legacy_public=1,privacy='public' WHERE id=?", (VIDEO,))


def prove_duplicate_binding_and_copy_receipts(connection: sqlite3.Connection) -> None:
    operation = uid(901)
    connection.execute("BEGIN")
    connection.execute(
        SQL["operation_insert"],
        (
            operation,
            "cap-v1-e6a882aeeffaa4f6",
            "video_duplicate",
            ACTOR,
            ORG,
            VIDEO,
            LEGACY_VIDEO,
            digest("duplicate-key"),
            digest("duplicate-request"),
            DUPLICATE,
            LEGACY_DUPLICATE,
            PREFIX,
            DUPLICATE_PREFIX,
            "{}",
            "claimed",
            NOW,
        ),
    )
    connection.execute(SQL["duplicate_video_insert"], (VIDEO, DUPLICATE, NOW, operation, ACTOR))
    connection.execute(SQL["duplicate_alias_insert"], (LEGACY_DUPLICATE, DUPLICATE, NOW))
    connection.execute(
        SQL["duplicate_media_update"],
        (DUPLICATE, DUPLICATE_PREFIX, "webMP4", None, NOW),
    )
    connection.execute(SQL["operation_storage_pending"], (operation,))
    connection.execute("COMMIT")

    duplicate = connection.execute("SELECT * FROM videos WHERE id=?", (DUPLICATE,)).fetchone()
    assert duplicate["owner_id"] == ACTOR and duplicate["legacy_password_hash"] is None
    assert duplicate["legacy_settings_json"] == '{"source":"settings"}'
    media = connection.execute(
        "SELECT * FROM legacy_mobile_cap_media_v1 WHERE mapped_video_id=?", (DUPLICATE,)
    ).fetchone()
    assert media["object_prefix"] == DUPLICATE_PREFIX

    source_key = PREFIX + "result.mp4"
    destination_key = DUPLICATE_PREFIX + "result.mp4"
    assert connection.execute(SQL["copy_receipt_exists"], (operation, source_key)).fetchone()[0] == 0
    connection.execute(
        SQL["copy_receipt_insert"],
        (operation, source_key, destination_key, "etag-v1", 1234, NOW + 1),
    )
    connection.execute(
        SQL["copy_receipt_insert"],
        (operation, source_key, destination_key, "etag-v1", 1234, NOW + 1),
    )
    assert connection.execute(SQL["copy_receipt_exists"], (operation, source_key)).fetchone()[0] == 1
    connection.execute(SQL["operation_complete"], (operation, "{}", NOW + 2))
    assert (
        connection.execute(
            "SELECT state FROM legacy_video_lifecycle_operations_v1 WHERE operation_id=?",
            (operation,),
        ).fetchone()[0]
        == "complete"
    )
    try:
        connection.execute(
            "UPDATE legacy_video_lifecycle_copy_receipts_v1 SET source_bytes=99 WHERE operation_id=?",
            (operation,),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_video_lifecycle_copy_receipt_immutable_v1" in str(error)
    else:
        raise AssertionError("copy receipt accepted mutation")


def prove_delete_is_tombstone_then_storage_pending(connection: sqlite3.Connection) -> None:
    operation = uid(902)
    connection.execute("BEGIN")
    connection.execute(
        SQL["operation_insert"],
        (
            operation,
            "cap-v1-1e909cc023a9c4a7",
            "video_delete",
            ACTOR,
            ORG,
            VIDEO,
            LEGACY_VIDEO,
            digest("delete-key"),
            digest("delete-request"),
            None,
            None,
            PREFIX,
            None,
            "{}",
            "claimed",
            NOW + 10,
        ),
    )
    connection.execute(SQL["delete_tombstone"], (VIDEO, ACTOR, NOW + 10, operation))
    connection.execute(SQL["delete_postcondition_assert"], (operation, VIDEO, ACTOR))
    connection.execute(SQL["operation_storage_pending"], (operation,))
    connection.execute("COMMIT")
    video = connection.execute("SELECT state,deleted_at_ms FROM videos WHERE id=?", (VIDEO,)).fetchone()
    assert video["state"] == "deleted" and video["deleted_at_ms"] == NOW + 10
    assert (
        connection.execute(
            "SELECT state FROM legacy_video_lifecycle_operations_v1 WHERE operation_id=?",
            (operation,),
        ).fetchone()[0]
        == "storage_pending"
    ), "D1 tombstone must precede R2 cleanup"
    connection.execute(SQL["operation_complete"], (operation, "{}", NOW + 11))


def prove_organization_pointer_swap_and_authority(connection: sqlite3.Connection) -> None:
    assert connection.execute(SQL["organization_admin_snapshot"], (ACTOR, ORG)).fetchone()
    assert connection.execute(SQL["organization_admin_snapshot"], (FOREIGN, ORG)).fetchone() is None
    operation = uid(903)
    icon = f"organizations/{ORG}/{operation}.png"
    binding = json.dumps(
        {"rpc_id": "rpc-icon", "new_icon_key": icon, "old_icon_key": None},
        separators=(",", ":"),
    )
    connection.execute("BEGIN")
    connection.execute(
        SQL["operation_insert"],
        (
            operation,
            "cap-v1-e32af2138aa62c8d",
            "organisation_update",
            ACTOR,
            ORG,
            None,
            None,
            digest("icon-key"),
            digest("icon-request"),
            None,
            None,
            None,
            None,
            binding,
            "claimed",
            NOW + 20,
        ),
    )
    connection.execute(SQL["organization_icon_update"], (ORG, ACTOR, icon, operation, NOW + 20))
    connection.execute(SQL["operation_storage_pending"], (operation,))
    connection.execute("COMMIT")
    organization = connection.execute(
        "SELECT legacy_icon_key,legacy_desktop_icon_url FROM organizations WHERE id=?", (ORG,)
    ).fetchone()
    assert organization["legacy_icon_key"] == icon
    assert organization["legacy_desktop_icon_url"] == icon
    connection.execute(SQL["operation_complete"], (operation, binding, NOW + 21))


def prove_atomic_instant_receipt_and_replay_conflict(connection: sqlite3.Connection) -> None:
    operation = uid(904)
    result = json.dumps(
        {
            "id": LEGACY_INSTANT,
            "shareUrl": f"https://frame.example/s/{LEGACY_INSTANT}",
            "upload": {"type": "put", "url": "https://r2.example/upload", "headers": {}},
        },
        separators=(",", ":"),
    )
    connection.execute("BEGIN")
    connection.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,source_object_key,created_at_ms,updated_at_ms,
             organization_id,privacy,legacy_public,last_operation_id
           ) VALUES(?,?,'Cap Recording','uploading',?, ?, ?, ?,'public',1,?)""",
        (INSTANT, ACTOR, INSTANT_PREFIX + "result.mp4", NOW + 30, NOW + 30, ORG, operation),
    )
    connection.execute(SQL["duplicate_alias_insert"], (LEGACY_INSTANT, INSTANT, NOW + 30))
    connection.execute(
        SQL["operation_insert"],
        (
            operation,
            "cap-v1-7b4e8210491e549d",
            "video_instant_create",
            ACTOR,
            ORG,
            INSTANT,
            LEGACY_INSTANT,
            digest("instant-rpc-key"),
            digest("instant-rpc-request"),
            None,
            None,
            INSTANT_PREFIX,
            None,
            result,
            "claimed",
            NOW + 30,
        ),
    )
    connection.execute(SQL["operation_complete"], (operation, result, NOW + 30))
    connection.execute("COMMIT")
    replay = connection.execute(
        SQL["operation_by_key"],
        ("cap-v1-7b4e8210491e549d", ACTOR, digest("instant-rpc-key")),
    ).fetchone()
    assert replay["state"] == "complete" and json.loads(replay["result_json"])["id"] == LEGACY_INSTANT
    try:
        connection.execute(
            SQL["operation_insert"],
            (
                uid(905),
                "cap-v1-7b4e8210491e549d",
                "video_instant_create",
                ACTOR,
                ORG,
                None,
                None,
                digest("instant-rpc-key"),
                digest("different-request"),
                None,
                None,
                None,
                None,
                None,
                "claimed",
                NOW + 31,
            ),
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("same Effect request key accepted different bytes")


def prove_immutable_replay_and_foreign_keys(connection: sqlite3.Connection) -> None:
    operation = uid(901)
    try:
        connection.execute(
            "UPDATE legacy_video_lifecycle_operations_v1 SET request_digest=? WHERE operation_id=?",
            (digest("tampered"), operation),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_video_lifecycle_operation_immutable_v1" in str(error)
    else:
        raise AssertionError("immutable replay receipt accepted tampering")
    assert connection.execute("PRAGMA foreign_key_check").fetchall() == [], "foreign_key_check"


def main() -> None:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    assert fixture["schema_version"] == "frame.legacy-video-lifecycle.v1"
    assert len(fixture["operations"]) == 8
    connection = database()
    seed(connection)
    prove_authority_and_og_non_disclosure(connection)
    prove_duplicate_binding_and_copy_receipts(connection)
    prove_delete_is_tombstone_then_storage_pending(connection)
    prove_organization_pointer_swap_and_authority(connection)
    prove_atomic_instant_receipt_and_replay_conflict(connection)
    prove_immutable_replay_and_foreign_keys(connection)
    print("legacy video lifecycle SQLite conformance passed")


if __name__ == "__main__":
    main()
