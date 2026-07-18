#!/usr/bin/env python3
"""SQLite proof for Cap's mobile create/progress/complete upload lifecycle."""

from __future__ import annotations

import hashlib
import json
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_mobile_uploads"
FIXTURE = ROOT / "fixtures/api-parity/v1/mobile-uploads.json"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}
NOW = 1_735_787_045_006
ACTOR = "00000000-0000-7000-8000-000000000001"
FOREIGN = "00000000-0000-7000-8000-000000000002"
ORG = "00000000-0000-7000-8000-000000000101"
FOREIGN_ORG = "00000000-0000-7000-8000-000000000102"
STORAGE = "00000000-0000-7000-8000-000000000201"
FOLDER = "00000000-0000-7000-8000-000000000301"
VIDEO = "00000000-0000-7000-8000-000000000401"
UPLOAD = "00000000-0000-7000-8000-000000000501"
LEGACY_ACTOR = "0123456789abcde"
LEGACY_FOREIGN = "0123456789abcdf"
LEGACY_ORG = "0123456789abcdg"
LEGACY_FOREIGN_ORG = "0123456789abcdh"
LEGACY_FOLDER = "0123456789abcdj"
LEGACY_VIDEO = "0123456789abcdk"
RAW_KEY = f"{LEGACY_ACTOR}/{LEGACY_VIDEO}/raw-upload.mov"
SOURCE_KEY = f"{LEGACY_ACTOR}/{LEGACY_VIDEO}/result.mp4"


def identifier(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:", isolation_level=None)
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def seed(connection: sqlite3.Connection) -> None:
    connection.executemany(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
        [
            (ACTOR, "actor@example.test", "Mobile Owner", 1, 1),
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
        [(ORG, ACTOR, "Mobile Org"), (FOREIGN_ORG, FOREIGN, "Foreign Org")],
    )
    connection.executemany(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,'owner','active',0,1,1)""",
        [(ORG, ACTOR), (FOREIGN_ORG, FOREIGN)],
    )
    connection.executemany(
        """INSERT INTO legacy_user_account_organization_ids_v1(
             organization_id,legacy_organization_id,recorded_at_ms,last_operation_id
           ) VALUES(?,?,1,?)""",
        [
            (ORG, LEGACY_ORG, identifier(801)),
            (FOREIGN_ORG, LEGACY_FOREIGN_ORG, identifier(802)),
        ],
    )
    connection.executemany(
        "UPDATE users SET active_organization_id=?,default_organization_id=? WHERE id=?",
        [(ORG, ORG, ACTOR), (FOREIGN_ORG, FOREIGN_ORG, FOREIGN)],
    )
    capabilities = json.dumps(
        {"schema_version": 1, "single_put": 1, "multipart": 1}, separators=(",", ":")
    )
    connection.execute(
        """INSERT INTO storage_integrations(
             id,organization_id,owner_user_id,provider,state,capabilities_json,
             credential_ciphertext,created_at_ms,updated_at_ms,capabilities_checksum
           ) VALUES(?,?,?,'r2','active',?,'sealed-test',1,1,?)""",
        (STORAGE, ORG, ACTOR, capabilities, digest(capabilities)),
    )
    connection.execute(
        """INSERT INTO folders(
             id,organization_id,space_id,parent_id,created_by_user_id,name,is_public,
             settings_json,created_at_ms,updated_at_ms,legacy_folder_id,legacy_name,
             legacy_color,legacy_scope_kind,legacy_scope_id
           ) VALUES(?,?,NULL,NULL,?,'Mobile',0,'{}',1,1,?,'Mobile','normal','personal',NULL)""",
        (FOLDER, ORG, ACTOR, LEGACY_FOLDER),
    )


def create_mobile_upload(connection: sqlite3.Connection) -> None:
    authority = connection.execute(
        SQL["create_authority"], (ACTOR, LEGACY_ORG, LEGACY_FOLDER)
    ).fetchall()
    assert len(authority) == 1
    assert authority[0]["legacy_actor_id"] == LEGACY_ACTOR
    assert authority[0]["folder_id"] == FOLDER
    assert (
        connection.execute(
            SQL["create_authority"], (ACTOR, LEGACY_FOREIGN_ORG, None)
        ).fetchone()
        is None
    )
    operation = identifier(901)
    request = digest("create-mobile-upload")
    connection.execute("BEGIN")
    connection.execute(
        SQL["create_authority_assert"],
        (operation, ACTOR, LEGACY_ACTOR, ORG, STORAGE, FOLDER),
    )
    connection.execute(
        SQL["create_video_insert"],
        (
            VIDEO,
            ACTOR,
            "Holiday Clip",
            12_500,
            NOW,
            ORG,
            FOLDER,
            "public",
            operation,
            1,
            12.5,
            1920.0,
            1080.0,
            SOURCE_KEY,
        ),
    )
    connection.execute(SQL["create_alias_insert"], (LEGACY_VIDEO, VIDEO, NOW))
    connection.execute(
        SQL["create_upload_insert"],
        (
            UPLOAD,
            ORG,
            VIDEO,
            100,
            f"legacy-mobile-upload:{LEGACY_VIDEO}",
            RAW_KEY,
            "video/quicktime",
            NOW,
            operation,
        ),
    )
    connection.execute(
        SQL["create_record_insert"],
        (
            VIDEO,
            LEGACY_VIDEO,
            ACTOR,
            LEGACY_ACTOR,
            ORG,
            STORAGE,
            UPLOAD,
            FOLDER,
            RAW_KEY,
            "Holiday Clip.MOV",
            "video/quicktime",
            100,
            12.5,
            1920.0,
            1080.0,
            60.0,
            NOW,
            operation,
        ),
    )
    connection.execute(
        SQL["operation_insert"],
        (
            operation,
            "cap-v1-b0116dd82b010477",
            "create",
            ACTOR,
            ORG,
            VIDEO,
            LEGACY_VIDEO,
            request,
            "complete",
            NOW,
        ),
    )
    connection.execute(
        SQL["create_postcondition_assert"],
        (operation, VIDEO, LEGACY_VIDEO, ACTOR, ORG, RAW_KEY, UPLOAD),
    )
    connection.execute(SQL["assertion_cleanup"], (operation,))
    connection.execute("COMMIT")
    record = connection.execute(
        "SELECT * FROM legacy_mobile_upload_records_v1 WHERE mapped_video_id=?", (VIDEO,)
    ).fetchone()
    assert record["raw_file_key"] == RAW_KEY and record["lifecycle_state"] == "uploading"
    media = connection.execute(
        "SELECT * FROM legacy_mobile_cap_media_v1 WHERE mapped_video_id=?", (VIDEO,)
    ).fetchone()
    assert media["legacy_video_id"] == LEGACY_VIDEO and media["source_type"] == "webMP4"
    progress = connection.execute(
        "SELECT * FROM legacy_mobile_cap_uploads_v1 WHERE mapped_video_id=?", (VIDEO,)
    ).fetchone()
    assert progress["uploaded"] == 0.0 and progress["total"] == 100.0


def prove_progress_and_non_disclosure(connection: sqlite3.Connection) -> None:
    seed(connection)
    create_mobile_upload(connection)
    assert (
        connection.execute(SQL["progress_snapshot"], (FOREIGN, LEGACY_VIDEO)).fetchone()
        is None
    )
    operation = identifier(902)
    connection.execute("BEGIN")
    connection.execute(
        SQL["progress_authority_assert"],
        (operation, VIDEO, ACTOR, LEGACY_VIDEO, UPLOAD),
    )
    # Runtime already applied Math.trunc/max/min: 12.9/10.8 becomes 10/10.
    connection.execute(SQL["progress_update"], (VIDEO, UPLOAD, 10, 10, NOW + 1, operation))
    connection.execute(
        SQL["operation_insert"],
        (
            operation,
            "cap-v1-62469fe03e030052",
            "progress",
            ACTOR,
            ORG,
            VIDEO,
            LEGACY_VIDEO,
            digest("progress-10-10"),
            "complete",
            NOW + 1,
        ),
    )
    connection.execute(
        SQL["progress_postcondition_assert"], (operation, UPLOAD, VIDEO, 10, 10)
    )
    connection.execute(SQL["assertion_cleanup"], (operation,))
    connection.execute("COMMIT")
    upload = connection.execute("SELECT * FROM video_uploads WHERE id=?", (UPLOAD,)).fetchone()
    projection = connection.execute(
        "SELECT * FROM legacy_mobile_cap_uploads_v1 WHERE mapped_video_id=?", (VIDEO,)
    ).fetchone()
    assert upload["received_bytes"] == 10 and upload["expected_bytes"] == 10
    assert projection["uploaded"] == 10.0 and projection["total"] == 10.0


def prove_completion_is_durable_and_provider_gated(connection: sqlite3.Connection) -> None:
    seed(connection)
    create_mobile_upload(connection)
    assert (
        connection.execute(SQL["complete_snapshot"], (FOREIGN, LEGACY_VIDEO)).fetchone()
        is None
    )
    snapshot = connection.execute(
        SQL["complete_snapshot"], (ACTOR, LEGACY_VIDEO)
    ).fetchone()
    assert snapshot["raw_file_key"] == RAW_KEY and snapshot["intent_state"] is None
    operation = identifier(903)
    observed = 100
    connection.execute("BEGIN")
    connection.execute(
        SQL["complete_authority_assert"],
        (operation, VIDEO, LEGACY_VIDEO, ACTOR, ORG, RAW_KEY),
    )
    connection.execute(
        SQL["operation_insert"],
        (
            operation,
            "cap-v1-b43b6ede64a73798",
            "complete",
            ACTOR,
            ORG,
            VIDEO,
            LEGACY_VIDEO,
            digest("complete-mobile-upload"),
            "provider_pending",
            NOW + 2,
        ),
    )
    connection.execute(
        SQL["complete_upload_bytes"], (UPLOAD, VIDEO, ORG, observed, NOW + 2, operation)
    )
    connection.execute(
        SQL["complete_record_pending"], (VIDEO, ACTOR, operation, NOW + 2)
    )
    connection.execute(
        SQL["complete_intent_insert"],
        (VIDEO, operation, ACTOR, ORG, RAW_KEY, observed, observed, NOW + 2),
    )
    connection.execute(SQL["complete_pending_assert"], (operation, VIDEO, observed))
    connection.execute(SQL["assertion_cleanup"], (operation,))
    connection.execute("COMMIT")
    pending = connection.execute(
        SQL["complete_snapshot"], (ACTOR, LEGACY_VIDEO)
    ).fetchone()
    assert pending["lifecycle_state"] == "provider_pending"
    assert pending["intent_state"] == "provider_pending"
    # Provider intent is not fabricated workflow submission or processing.
    assert connection.execute("SELECT COUNT(*) FROM media_jobs").fetchone()[0] == 0
    cap_progress = connection.execute(
        "SELECT * FROM legacy_mobile_cap_uploads_v1 WHERE mapped_video_id=?", (VIDEO,)
    ).fetchone()
    assert cap_progress["phase"] == "uploading"
    try:
        connection.execute(
            "UPDATE legacy_mobile_upload_processing_intents_v1 SET observed_bytes=101 WHERE mapped_video_id=?",
            (VIDEO,),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_mobile_upload_processing_intent_transition_v1" in str(error)
    else:
        raise AssertionError("provider-pending intent became mutable")


def prove_fixture_and_source_manifests() -> None:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    assert fixture["reference_commit"] == "6ba69561ac86b8efdb17616d6727f9638015546b"
    assert [operation["id"] for operation in fixture["operations"]] == [
        "cap-v1-b0116dd82b010477",
        "cap-v1-b43b6ede64a73798",
        "cap-v1-62469fe03e030052",
    ]
    assert fixture["completion_contract"]["protected_gates"] == ["provider_execution"]
    assert "success=true" in fixture["completion_contract"]["production_behavior"]


def main() -> None:
    prove_fixture_and_source_manifests()
    for proof in (
        prove_progress_and_non_disclosure,
        prove_completion_is_durable_and_provider_gated,
    ):
        connection = database()
        try:
            proof(connection)
        finally:
            connection.close()
    print("legacy mobile uploads SQLite conformance: ok")


if __name__ == "__main__":
    main()
