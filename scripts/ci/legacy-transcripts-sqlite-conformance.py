#!/usr/bin/env python3
"""Provider-free SQLite proof for transcript policy, retry, R2 receipts, and provider outbox."""

from __future__ import annotations

import json
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
QUERIES = ROOT / "apps" / "control-plane" / "queries" / "legacy_transcripts"

OWNER = "00000000-0000-4000-8000-000000000001"
VIEWER = "00000000-0000-4000-8000-000000000002"
ORG = "10000000-0000-4000-8000-000000000001"
SPACE = "20000000-0000-4000-8000-000000000001"
VIDEO = "30000000-0000-4000-8000-000000000001"
OWNER_ALIAS = "0123456789abcde"
VIDEO_ALIAS = "1123456789abcde"
EDIT_OPERATION = "40000000-0000-4000-8000-000000000001"
TRANSLATE_OPERATION = "40000000-0000-4000-8000-000000000002"
DIGEST_A = "a" * 64
DIGEST_B = "b" * 64
DIGEST_C = "c" * 64
DIGEST_D = "d" * 64


def sql(name: str) -> str:
    return (QUERIES / name).read_text(encoding="utf-8")


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:")
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def seed(connection: sqlite3.Connection) -> None:
    for user_id, email in [(OWNER, "owner@example.test"), (VIEWER, "viewer@example.test")]:
        connection.execute(
            "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
            (user_id, email, email.split("@")[0], 1, 1),
        )
    connection.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,legacy_allowed_email_restriction,created_at_ms,updated_at_ms
           ) VALUES(?,?,?,'active','example.test',1,1)""",
        (ORG, OWNER, "Transcript org"),
    )
    for user_id, role in [(OWNER, "owner"), (VIEWER, "viewer")]:
        connection.execute(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,created_at_ms,updated_at_ms
               ) VALUES(?,?,?,'active',1,1)""",
            (ORG, user_id, role),
        )
    connection.execute(
        """INSERT INTO spaces(
             id,organization_id,created_by_user_id,name,created_at_ms,updated_at_ms,
             legacy_password_hash
           ) VALUES(?,?,?,'Transcript space',1,1,?)""",
        (SPACE, ORG, OWNER, "Q" * 64),
    )
    connection.execute(
        "INSERT INTO space_members(space_id,user_id,role,created_at_ms,updated_at_ms) VALUES(?,?,'viewer',1,1)",
        (SPACE, VIEWER),
    )
    connection.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,organization_id,privacy,legacy_public,
             legacy_password_hash,created_at_ms,updated_at_ms
           ) VALUES(?,?,'Transcript','ready',?,'public',1,?,1,1)""",
        (VIDEO, OWNER, ORG, "P" * 64),
    )
    connection.execute(
        "INSERT INTO space_videos(space_id,video_id,added_by_user_id,added_at_ms) VALUES(?,?,?,1)",
        (SPACE, VIDEO, OWNER),
    )
    connection.execute(
        """INSERT INTO legacy_collaboration_user_aliases_v1(
             legacy_user_id,mapped_user_id,provenance,created_at_ms,refreshed_at_ms
           ) VALUES(?,?,'cap_backfill',1,1)""",
        (OWNER_ALIAS, OWNER),
    )
    connection.execute(
        """INSERT INTO legacy_collaboration_video_aliases_v1(
             legacy_video_id,mapped_video_id,provenance,created_at_ms
           ) VALUES(?,?,'cap_backfill',1)""",
        (VIDEO_ALIAS, VIDEO),
    )
    connection.execute(
        """UPDATE legacy_mobile_cap_media_v1
           SET object_prefix=?, source_type='webMP4', transcription_status='COMPLETE'
           WHERE mapped_video_id=?""",
        (f"{OWNER_ALIAS}/{VIDEO_ALIAS}/", VIDEO),
    )
    connection.commit()


def insert_operation(
    connection: sqlite3.Connection,
    operation_id: str,
    source_id: str,
    kind: str,
    actor_id: str | None,
    object_key: str,
    target_language: str | None,
    entry_id: int | None,
    replacement_text: str | None,
) -> None:
    connection.execute(
        sql("operation_insert.sql"),
        (
            operation_id,
            source_id,
            kind,
            DIGEST_A if actor_id else DIGEST_B,
            actor_id,
            VIDEO,
            VIDEO_ALIAS,
            DIGEST_C,
            DIGEST_D,
            object_key,
            target_language,
            entry_id,
            replacement_text,
            "claimed",
            10,
        ),
    )


def main() -> None:
    connection = database()
    seed(connection)

    video = connection.execute(sql("video_authority.sql"), (VIDEO_ALIAS,)).fetchone()
    assert video["mapped_video_id"] == VIDEO
    assert video["object_prefix"] == f"{OWNER_ALIAS}/{VIDEO_ALIAS}/"
    assert video["transcription_status"] == "COMPLETE"
    assert video["allowed_email_restriction"] == "example.test"

    access = connection.execute(sql("explicit_access.sql"), (VIDEO, VIEWER)).fetchone()
    assert access["allowed"] == 1
    candidates = connection.execute(sql("password_candidates.sql"), (VIDEO,)).fetchall()
    assert [(row["password_hash"], row["ordinal"]) for row in candidates] == [
        ("P" * 64, 0),
        ("Q" * 64, 1),
    ]

    connection.execute(sql("retry_status_reset.sql"), (VIDEO, 20, OWNER))
    assert connection.execute(
        "SELECT transcription_status FROM legacy_mobile_cap_media_v1 WHERE mapped_video_id=?",
        (VIDEO,),
    ).fetchone()[0] is None

    original_key = f"{OWNER_ALIAS}/{VIDEO_ALIAS}/transcription.vtt"
    insert_operation(
        connection,
        EDIT_OPERATION,
        "cap-v1-3db394ae13895b46",
        "edit",
        OWNER,
        original_key,
        None,
        1,
        "replacement",
    )
    # Same key/request replays the original claim; a different request cannot
    # overwrite the immutable request digest.
    insert_operation(
        connection,
        "40000000-0000-4000-8000-000000000099",
        "cap-v1-3db394ae13895b46",
        "edit",
        OWNER,
        original_key,
        None,
        1,
        "replacement",
    )
    rows = connection.execute(
        sql("operation_by_key.sql"),
        ("cap-v1-3db394ae13895b46", DIGEST_A, VIDEO, DIGEST_C),
    ).fetchall()
    assert len(rows) == 1 and rows[0]["operation_id"] == EDIT_OPERATION

    connection.execute(
        sql("storage_receipt_insert.sql"),
        (EDIT_OPERATION, original_key, "before", "after", DIGEST_A, 32, 21),
    )
    connection.execute(
        sql("operation_storage_applied.sql"), (EDIT_OPERATION, 21, DIGEST_D)
    )
    result = json.dumps(
        {"success": True, "message": "Transcript entry updated successfully"},
        separators=(",", ":"),
    )
    connection.execute(
        sql("operation_complete.sql"), (EDIT_OPERATION, result, 22, DIGEST_D)
    )
    assert connection.execute(
        "SELECT state FROM legacy_transcript_operations_v1 WHERE operation_id=?",
        (EDIT_OPERATION,),
    ).fetchone()[0] == "complete"
    try:
        connection.execute(
            "UPDATE legacy_transcript_storage_receipts_v1 SET applied_etag='changed' WHERE operation_id=?",
            (EDIT_OPERATION,),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_transcript_receipt_immutable_v1" in str(error)
    else:
        raise AssertionError("storage receipt mutation unexpectedly succeeded")

    target_key = f"{OWNER_ALIAS}/{VIDEO_ALIAS}/transcription.es.vtt"
    insert_operation(
        connection,
        TRANSLATE_OPERATION,
        "cap-v1-6f6ece85bd786289",
        "translate",
        None,
        target_key,
        "es",
        None,
        None,
    )
    connection.execute(
        sql("translation_outbox_insert.sql"),
        (TRANSLATE_OPERATION, original_key, target_key, "es", 30),
    )
    connection.execute(
        sql("operation_provider_pending.sql"),
        (TRANSLATE_OPERATION, 30, DIGEST_D),
    )
    outbox = connection.execute(
        "SELECT state,model,target_language FROM legacy_transcript_translation_outbox_v1 WHERE operation_id=?",
        (TRANSLATE_OPERATION,),
    ).fetchone()
    assert tuple(outbox) == ("pending", "openai/gpt-oss-120b", "es")

    print(
        "legacy transcripts SQLite conformance passed: source alias/prefix projection, "
        "public-policy inputs, password order, owner retry, immutable R2 receipt, replay, "
        "and protected translation outbox"
    )


if __name__ == "__main__":
    main()
