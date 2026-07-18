#!/usr/bin/env python3
"""Provider-free SQLite proof for Cap extension instant recordings."""

from __future__ import annotations

import hashlib
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_extension_instant"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}
NOW = 1_721_174_400_000
ACTOR = "00000000-0000-7000-8000-000000000001"
OTHER = "00000000-0000-7000-8000-000000000002"
ORG = "00000000-0000-7000-8000-000000000101"
OTHER_ORG = "00000000-0000-7000-8000-000000000102"
STORAGE = "00000000-0000-7000-8000-000000000201"
FOLDER = "00000000-0000-7000-8000-000000000301"
LEGACY_FOLDER = "0123456789abcdf"
VIDEO = "00000000-0000-7000-8000-000000000401"
UPLOAD = "00000000-0000-7000-8000-000000000501"
ALIAS = "0123456789abcde"


def identifier(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def database() -> sqlite3.Connection:
    db = sqlite3.connect(":memory:")
    db.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        db.executescript(migration.read_text(encoding="utf-8"))
    return db


def seed(db: sqlite3.Connection) -> None:
    capabilities = '{"schema_version":1}'
    db.executemany(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
        [
            (ACTOR, "actor@example.test", "Actor", 1, 1),
            (OTHER, "other@example.test", "Other", 1, 1),
        ],
    )
    db.executemany(
        "INSERT INTO organizations(id,owner_id,name,status,created_at_ms,updated_at_ms) VALUES(?,?,?,'active',1,1)",
        [(ORG, ACTOR, "Frame"), (OTHER_ORG, OTHER, "Other")],
    )
    db.executemany(
        "INSERT INTO organization_members(organization_id,user_id,role,state,created_at_ms,updated_at_ms) VALUES(?,?,'owner','active',1,1)",
        [(ORG, ACTOR), (OTHER_ORG, OTHER)],
    )
    db.executemany(
        "UPDATE users SET active_organization_id=? WHERE id=?",
        [(ORG, ACTOR), (OTHER_ORG, OTHER)],
    )
    db.execute(
        """INSERT INTO storage_integrations(
             id,organization_id,owner_user_id,provider,state,capabilities_json,
             credential_ciphertext,created_at_ms,updated_at_ms,capabilities_checksum
           ) VALUES(?,?,?,'r2','active',?,'sealed-test-credential',1,1,?)""",
        (STORAGE, ORG, ACTOR, capabilities, hashlib.sha256(capabilities.encode()).hexdigest()),
    )
    db.execute(
        """INSERT INTO folders(
             id,organization_id,created_by_user_id,name,created_at_ms,updated_at_ms,
             legacy_folder_id,legacy_name,legacy_scope_kind,legacy_scope_id
           ) VALUES(?,?,?,'Recordings',1,1,?,'Recordings','personal',?)""",
        (FOLDER, ORG, ACTOR, LEGACY_FOLDER, None),
    )
    db.commit()


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def create(db: sqlite3.Connection) -> None:
    authority = db.execute(
        SQL["create_authority"], (ACTOR, ORG, LEGACY_FOLDER)
    ).fetchall()
    assert len(authority) == 1
    assert authority[0]["storage_integration_id"] == STORAGE
    assert authority[0]["folder_id"] == FOLDER
    assert db.execute(
        SQL["create_authority"], (ACTOR, OTHER_ORG, None)
    ).fetchone() is None
    assert db.execute(
        SQL["create_authority"], (ACTOR, ORG, "not-a-live-folder")
    ).fetchone() is None

    operation = identifier(601)
    key = f"{ACTOR}/{ALIAS}/result.mp4"
    prefix = f"{ACTOR}/{ALIAS}/"
    db.execute("BEGIN")
    try:
        db.execute(
            SQL["create_authority_assert"],
            (operation, ACTOR, ORG, FOLDER, STORAGE),
        )
        db.execute(
            SQL["create_video_insert"],
            (
                VIDEO,
                ACTOR,
                "Cap Recording - 17 July 2024",
                12_500,
                NOW,
                ORG,
                FOLDER,
                "public",
                operation,
                1,
                12.5,
                "1920x1080",
                1920.0,
                1080.0,
                "h264",
                "aac",
                1,
            ),
        )
        db.execute(SQL["create_alias_insert"], (ALIAS, VIDEO, NOW))
        db.execute(
            SQL["create_upload_insert"],
            (UPLOAD, ORG, VIDEO, f"legacy-extension-instant:{ALIAS}", key, NOW, operation, 1),
        )
        db.execute(
            SQL["create_recording_insert"],
            (ALIAS, VIDEO, UPLOAD, ORG, ACTOR, STORAGE, prefix, key, 1, NOW, operation),
        )
        db.execute(
            SQL["create_operation_insert"],
            (operation, ACTOR, ORG, ALIAS, VIDEO, digest("create"), NOW),
        )
        db.execute(
            SQL["create_postcondition_assert"],
            (operation, ALIAS, VIDEO, ACTOR, ORG, key, 1, UPLOAD),
        )
        db.execute(SQL["assertion_cleanup"], (operation,))
        db.commit()
    except Exception:
        db.rollback()
        raise

    video = db.execute(
        """SELECT id,owner_id,organization_id,folder_id,privacy,legacy_public,
                  legacy_duration_seconds,legacy_instant_recording,
                  legacy_instant_supports_progress
           FROM videos WHERE id=?""",
        (VIDEO,),
    ).fetchone()
    assert tuple(video) == (VIDEO, ACTOR, ORG, FOLDER, "public", 1, 12.5, 1, 1)
    assert tuple(
        db.execute(
            "SELECT legacy_video_id,mapped_video_id,provenance FROM legacy_collaboration_video_aliases_v1 WHERE legacy_video_id=?",
            (ALIAS,),
        ).fetchone()
    ) == (ALIAS, VIDEO, "native_generated")
    assert tuple(
        db.execute(
            "SELECT storage_prefix,source_object_key,lifecycle_state,storage_cleanup_state FROM legacy_extension_instant_recordings_v1 WHERE legacy_video_id=?",
            (ALIAS,),
        ).fetchone()
    ) == (prefix, key, "active", "not_requested")


def progress(
    db: sqlite3.Connection,
    number: int,
    actor: str,
    uploaded: int,
    total: int,
    updated_at: int,
    *,
    expect_failure: bool = False,
) -> None:
    snapshot = db.execute(SQL["progress_snapshot"], (ALIAS,)).fetchone()
    assert snapshot is not None
    operation = identifier(700 + number)
    generated_upload = identifier(800 + number)
    db.execute("BEGIN")
    try:
        db.execute(
            SQL["progress_authority_assert"],
            (operation, ALIAS, snapshot["mapped_video_id"], actor, snapshot["organization_id"]),
        )
        db.execute(
            SQL["progress_upload_insert"],
            (
                generated_upload,
                snapshot["organization_id"],
                snapshot["mapped_video_id"],
                total,
                f"legacy-extension-progress:{ALIAS}",
                snapshot["source_object_key"],
                NOW + number,
                updated_at,
                operation,
                1 if snapshot["upload_id"] is None else 0,
            ),
        )
        upload = snapshot["upload_id"] or generated_upload
        db.execute(
            SQL["progress_recording_claim_upload"],
            (ALIAS, snapshot["mapped_video_id"], upload, operation),
        )
        db.execute(
            SQL["progress_update"],
            (ALIAS, snapshot["mapped_video_id"], actor, min(uploaded, total), total, updated_at, operation),
        )
        db.execute(
            SQL["progress_operation_insert"],
            (
                operation,
                actor,
                snapshot["organization_id"],
                ALIAS,
                snapshot["mapped_video_id"],
                digest(f"progress-{number}"),
                min(uploaded, total),
                total,
                updated_at,
                NOW + number,
            ),
        )
        db.execute(SQL["assertion_cleanup"], (operation,))
        db.commit()
        if expect_failure:
            raise AssertionError("wrong-owner progress crossed tenant authority")
    except sqlite3.IntegrityError as error:
        db.rollback()
        if not expect_failure:
            raise
        assert "frame_legacy_extension_instant_assertion_failed_v1" in str(error)


def delete(db: sqlite3.Connection) -> None:
    snapshot = db.execute(SQL["delete_snapshot"], (ALIAS,)).fetchone()
    assert snapshot["actor_id"] == ACTOR
    operation = identifier(901)
    db.execute("BEGIN")
    try:
        db.execute(
            SQL["delete_authority_assert"],
            (operation, ALIAS, VIDEO, ACTOR, ORG),
        )
        db.execute(SQL["delete_mark"], (ALIAS, VIDEO, NOW + 100, operation))
        db.execute(
            SQL["delete_video_tombstone"], (VIDEO, ACTOR, NOW + 100, operation)
        )
        db.execute(
            SQL["delete_operation_insert"],
            (operation, ACTOR, ORG, ALIAS, VIDEO, digest("delete"), NOW + 100),
        )
        db.execute(SQL["assertion_cleanup"], (operation,))
        db.commit()
    except Exception:
        db.rollback()
        raise

    pending = db.execute(SQL["delete_snapshot"], (ALIAS,)).fetchone()
    assert pending["lifecycle_state"] == "deleting"
    assert pending["pending_operation_id"] == operation
    assert db.execute("SELECT deleted_at_ms FROM videos WHERE id=?", (VIDEO,)).fetchone()[0]

    # This transaction represents successful strongly-consistent R2 prefix
    # deletion. Alias/tombstone rows remain durable after finalization.
    db.execute("BEGIN")
    try:
        db.execute(
            SQL["delete_upload_abort"],
            (UPLOAD, ORG, NOW + 101, digest("abort"), operation),
        )
        db.execute(
            SQL["delete_cleanup_assert"], (operation, ALIAS, VIDEO, ACTOR)
        )
        db.execute(
            SQL["delete_finalize_recording"],
            (ALIAS, VIDEO, NOW + 101, operation),
        )
        db.execute(
            SQL["delete_finalize_operation"], (operation, NOW + 101)
        )
        db.execute(SQL["assertion_cleanup"], (operation,))
        db.commit()
    except Exception:
        db.rollback()
        raise
    assert db.execute(SQL["delete_snapshot"], (ALIAS,)).fetchone() is None
    assert tuple(
        db.execute(
            "SELECT lifecycle_state,storage_cleanup_state FROM legacy_extension_instant_recordings_v1 WHERE legacy_video_id=?",
            (ALIAS,),
        ).fetchone()
    ) == ("deleted", "complete")
    assert db.execute(
        "SELECT mapped_video_id FROM legacy_collaboration_video_aliases_v1 WHERE legacy_video_id=?",
        (ALIAS,),
    ).fetchone()[0] == VIDEO


def main() -> None:
    expected_queries = {
        "assertion_cleanup", "create_alias_insert", "create_authority",
        "create_authority_assert", "create_operation_insert",
        "create_postcondition_assert", "create_recording_insert",
        "create_upload_insert", "create_video_insert", "delete_authority_assert",
        "delete_cleanup_assert", "delete_finalize_operation",
        "delete_finalize_recording", "delete_mark", "delete_operation_insert",
        "delete_snapshot", "delete_upload_abort", "delete_video_tombstone",
        "progress_authority_assert", "progress_operation_insert",
        "progress_recording_claim_upload", "progress_snapshot", "progress_update",
        "progress_upload_insert",
    }
    assert set(SQL) == expected_queries
    db = database()
    seed(db)
    create(db)

    source_time = NOW + 10_000
    progress(db, 1, ACTOR, 75, 100, source_time)
    assert tuple(
        db.execute(
            "SELECT received_bytes,expected_bytes,updated_at_ms FROM video_uploads WHERE id=?",
            (UPLOAD,),
        ).fetchone()
    ) == (75, 100, source_time)
    progress(db, 2, ACTOR, 75, 100, source_time)  # equal retry converges
    progress(db, 3, ACTOR, 60, 100, source_time + 1)  # uploaded regression no-op
    progress(db, 4, ACTOR, 90, 90, source_time + 2)  # total regression no-op
    progress(db, 5, ACTOR, 130, 120, source_time + 3)  # clamp to total
    assert tuple(
        db.execute(
            "SELECT received_bytes,expected_bytes,updated_at_ms FROM video_uploads WHERE id=?",
            (UPLOAD,),
        ).fetchone()
    ) == (120, 120, source_time + 3)
    progress(db, 6, ACTOR, 120, 130, source_time - 1)  # stale timestamp no-op
    progress(db, 7, OTHER, 125, 130, source_time + 4, expect_failure=True)
    assert [
        row[0]
        for row in db.execute(
            "SELECT applied FROM legacy_extension_instant_operations_v1 WHERE action='progress' ORDER BY created_at_ms"
        )
    ] == [1, 0, 0, 0, 1, 0]

    delete(db)
    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
    print(
        "legacy extension instant-recordings SQLite conformance passed: durable NanoID alias, "
        "native UUID/R2 authority, tenant isolation, monotonic progress, equal retry convergence, "
        "two-phase prefix cleanup, preserved tombstone, and foreign_key_check"
    )


if __name__ == "__main__":
    main()
