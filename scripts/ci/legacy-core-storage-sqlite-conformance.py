#!/usr/bin/env python3
"""SQLite proof for Cap core R2 storage, upload, and provider-gated finalization."""

from __future__ import annotations

import hashlib
import json
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_core_storage"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}
NOW = 1_735_787_045_006
ACTOR = "00000000-0000-7000-8000-000000000001"
FOREIGN = "00000000-0000-7000-8000-000000000002"
ORG = "00000000-0000-7000-8000-000000000101"
OTHER_ORG = "00000000-0000-7000-8000-000000000102"
INTEGRATION = "00000000-0000-7000-8000-000000000201"
VIDEO = "00000000-0000-7000-8000-000000000301"
LEGACY_ACTOR = "0123456789abcdf"
LEGACY_VIDEO = "0123456789abcde"
PREFIX = f"{LEGACY_ACTOR}/{LEGACY_VIDEO}/"


def uid(number: int) -> str:
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
            (ACTOR, "owner@example.test", "Owner", 1, 1),
            (FOREIGN, "foreign@example.test", "Foreign", 1, 1),
        ],
    )
    connection.executemany(
        """INSERT INTO legacy_collaboration_user_aliases_v1(
             legacy_user_id,mapped_user_id,image_url,provenance,created_at_ms,refreshed_at_ms
           ) VALUES(?,?,NULL,'native_generated',1,1)""",
        [(LEGACY_ACTOR, ACTOR), ("0123456789abcdg", FOREIGN)],
    )
    connection.executemany(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
           ) VALUES(?,?,'Storage Org','active','{}',1,1)""",
        [(ORG, ACTOR), (OTHER_ORG, FOREIGN)],
    )
    connection.executemany(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,created_at_ms,updated_at_ms
           ) VALUES(?,?,'owner','active',1,1)""",
        [(ORG, ACTOR), (OTHER_ORG, FOREIGN)],
    )
    connection.execute(
        "UPDATE users SET active_organization_id=?,default_organization_id=? WHERE id=?",
        (ORG, ORG, ACTOR),
    )
    connection.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,source_object_key,playback_object_key,duration_ms,
             created_at_ms,updated_at_ms,organization_id,privacy,legacy_public
           ) VALUES(?,?,'Core Storage','ready',?,?,12000,?,?,?,'private',0)""",
        (
            VIDEO,
            ACTOR,
            f"{PREFIX}result.mp4",
            f"{PREFIX}result.mp4",
            NOW - 10_000,
            NOW - 9_000,
            ORG,
        ),
    )
    connection.execute(
        """INSERT INTO legacy_collaboration_video_aliases_v1(
             legacy_video_id,mapped_video_id,provenance,created_at_ms
           ) VALUES(?,?,'native_generated',?)""",
        (LEGACY_VIDEO, VIDEO, NOW - 10_000),
    )
    capabilities = json.dumps(
        {"schema_version": 1, "single_put": 1, "multipart": 1},
        separators=(",", ":"),
    )
    connection.execute(
        """INSERT INTO storage_integrations(
             id,organization_id,owner_user_id,provider,state,capabilities_json,
             credential_ciphertext,created_at_ms,updated_at_ms,capabilities_checksum
           ) VALUES(?,?,?,'r2','active',?,'sealed-test',1,1,?)""",
        (INTEGRATION, ORG, ACTOR, capabilities, digest(capabilities)),
    )
    # The alias trigger created this projection before storage integration
    # insertion. Preserve the historical Cap owner alias, not the native UUID.
    connection.execute(
        """UPDATE legacy_mobile_cap_media_v1
           SET object_prefix=?,source_type='desktopSegments',updated_at_ms=?
           WHERE mapped_video_id=?""",
        (PREFIX, NOW - 8_000, VIDEO),
    )


def insert_operation(
    connection: sqlite3.Connection,
    operation_id: str,
    source_id: str,
    kind: str,
    request: str,
) -> None:
    connection.execute(
        SQL["operation_insert"],
        (
            operation_id,
            source_id,
            kind,
            ACTOR,
            ORG,
            VIDEO,
            LEGACY_VIDEO,
            digest(f"idempotency:{operation_id}"),
            1,
            digest(request),
            None,
            NOW,
        ),
    )


def insert_multipart(
    connection: sqlite3.Connection,
    external_id: str,
    provider_id: str,
    operation_id: str,
    subpath: str,
) -> None:
    connection.execute(
        SQL["multipart_insert"],
        (
            external_id,
            provider_id,
            operation_id,
            ACTOR,
            ORG,
            VIDEO,
            LEGACY_VIDEO,
            INTEGRATION,
            PREFIX,
            subpath,
            f"{PREFIX}{subpath}",
            "video/mp4",
            NOW,
            NOW + 86_400_000,
        ),
    )


def prove_authority_and_historical_prefix(connection: sqlite3.Connection) -> None:
    owner = connection.execute(SQL["owner_authority"], (ACTOR, LEGACY_VIDEO)).fetchall()
    assert len(owner) == 1
    assert owner[0]["object_prefix"] == PREFIX
    assert owner[0]["supports_single_put"] == 1
    assert owner[0]["supports_multipart"] == 1
    assert owner[0]["organization_owner_has_pro_seat"] == 0
    assert connection.execute(
        SQL["owner_authority"], (FOREIGN, LEGACY_VIDEO)
    ).fetchone() is None
    # Private video: exact token bit or owner admits; foreign/anonymous does not.
    assert connection.execute(
        SQL["read_authority"], (LEGACY_VIDEO, None, 0)
    ).fetchone() is None
    assert connection.execute(
        SQL["read_authority"], (LEGACY_VIDEO, ACTOR, 0)
    ).fetchone()["mapped_video_id"] == VIDEO
    assert connection.execute(
        SQL["read_authority"], (LEGACY_VIDEO, None, 1)
    ).fetchone()["mapped_video_id"] == VIDEO


def prove_initiate_and_replay_are_immutable(connection: sqlite3.Connection) -> None:
    operation = uid(401)
    external = uid(402)
    insert_operation(
        connection,
        operation,
        "cap-v1-f47512c6177fa691",
        "multipart_initiate",
        "initiate",
    )
    insert_multipart(connection, external, "provider-secret-1", operation, "result.mp4")
    result = json.dumps({"uploadId": external, "provider": "s3"}, separators=(",", ":"))
    connection.execute(
        SQL["operation_complete"], (operation, digest("initiate"), result, NOW + 1)
    )
    replay = connection.execute(
        SQL["operation_replay"],
        ("cap-v1-f47512c6177fa691", ACTOR, digest(f"idempotency:{operation}")),
    ).fetchone()
    assert replay["state"] == "complete"
    assert "provider-secret-1" not in replay["result_binding_json"]
    session = connection.execute(SQL["multipart_by_initiate"], (operation,)).fetchone()
    assert session["provider_upload_id"] == "provider-secret-1"
    assert session["object_prefix"] == PREFIX
    assert session["object_key"] == f"{PREFIX}result.mp4"
    try:
        connection.execute(
            "UPDATE legacy_core_storage_operations_v1 SET request_digest=? WHERE operation_id=?",
            (digest("tampered"), operation),
        )
        raise AssertionError("operation request binding was mutable")
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_core_storage_operation_immutable_v1" in str(error)
    try:
        connection.execute(
            "UPDATE legacy_core_storage_multipart_v1 SET object_prefix=? WHERE external_upload_id=?",
            (f"{ACTOR}/{LEGACY_VIDEO}/", external),
        )
        raise AssertionError("multipart historical prefix was mutable")
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_core_storage_multipart_transition_v1" in str(error)


def prove_completion_stops_at_provider_gate(connection: sqlite3.Connection) -> None:
    initiate = uid(410)
    complete = uid(411)
    external = uid(412)
    insert_operation(
        connection,
        initiate,
        "cap-v1-f47512c6177fa691",
        "multipart_initiate",
        "initiate-complete-case",
    )
    insert_multipart(connection, external, "provider-secret-2", initiate, "segments/video.m4s")
    insert_operation(
        connection,
        complete,
        "cap-v1-efc19423a62b7976",
        "multipart_complete",
        "complete",
    )
    connection.execute(
        SQL["multipart_part_insert"],
        (external, 1, '"etag-1"', 5, complete, NOW + 2),
    )
    connection.execute(
        SQL["multipart_part_insert"],
        (external, 2, '"etag-2"', 7, complete, NOW + 2),
    )
    parts_digest = digest("parts")
    connection.execute(
        SQL["multipart_mark_completion"],
        (external, ACTOR, complete, 12, parts_digest, NOW + 2),
    )
    pending = json.dumps(
        {
            "providerGate": "provider_execution",
            "uploadId": external,
            "partsDigest": parts_digest,
            "expectedBytes": 12,
        },
        separators=(",", ":"),
    )
    connection.execute(
        SQL["operation_effect_pending"], (complete, digest("complete"), pending)
    )
    session = connection.execute(SQL["multipart_select"], (external, ACTOR)).fetchone()
    assert session["state"] == "completion_pending"
    assert session["expected_bytes"] == 12
    operation = connection.execute(
        SQL["operation_by_id"],
        (complete, ACTOR, "cap-v1-efc19423a62b7976"),
    ).fetchone()
    assert operation["state"] == "effect_pending"
    assert json.loads(operation["result_binding_json"])["providerGate"] == "provider_execution"
    assert connection.execute(
        "SELECT COUNT(*) FROM storage_objects WHERE object_key=?",
        (f"{PREFIX}segments/video.m4s",),
    ).fetchone()[0] == 0
    try:
        connection.execute(
            "DELETE FROM legacy_core_storage_multipart_parts_v1 WHERE external_upload_id=?",
            (external,),
        )
        raise AssertionError("provider evidence parts were deletable")
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_core_storage_part_immutable_v1" in str(error)


def prove_abort_two_phase_terminal_replay(connection: sqlite3.Connection) -> None:
    initiate = uid(420)
    abort = uid(421)
    external = uid(422)
    insert_operation(
        connection,
        initiate,
        "cap-v1-f47512c6177fa691",
        "multipart_initiate",
        "initiate-abort-case",
    )
    insert_multipart(connection, external, "provider-secret-3", initiate, "raw-upload.mp4")
    insert_operation(
        connection,
        abort,
        "cap-v1-f191ed86271608e3",
        "multipart_abort",
        "abort",
    )
    connection.execute(SQL["multipart_mark_abort"], (external, ACTOR, abort, NOW + 3))
    connection.execute(
        SQL["operation_effect_pending"],
        (abort, digest("abort"), '{"providerEffect":"abort"}'),
    )
    assert connection.execute(
        SQL["multipart_select"], (external, ACTOR)
    ).fetchone()["state"] == "abort_pending"
    pending_operation = connection.execute(
        SQL["operation_by_id"],
        (abort, ACTOR, "cap-v1-f191ed86271608e3"),
    ).fetchone()
    assert pending_operation["state"] == "effect_pending"
    assert pending_operation["request_digest"] == digest("abort")
    connection.execute(
        SQL["multipart_finish_abort"], (external, ACTOR, abort, NOW + 4)
    )
    connection.execute(
        SQL["operation_complete"],
        (abort, digest("abort"), '{"success":true}', NOW + 4),
    )
    session = connection.execute(SQL["multipart_select"], (external, ACTOR)).fetchone()
    assert session["state"] == "aborted"
    assert session["terminal_at_ms"] == NOW + 4
    assert connection.execute(
        "SELECT state FROM legacy_core_storage_operations_v1 WHERE operation_id=?", (abort,)
    ).fetchone()[0] == "complete"


def prove_signed_capability_is_no_overwrite_governed(connection: sqlite3.Connection) -> None:
    operation = uid(430)
    insert_operation(
        connection,
        operation,
        "cap-v1-7f87205cb7d39ee6",
        "signed",
        "signed",
    )
    key = f"{PREFIX}result.mp4"
    connection.execute(
        SQL["object_intent_insert"],
        (
            uid(431),
            key,
            operation,
            ACTOR,
            ORG,
            VIDEO,
            LEGACY_VIDEO,
            INTEGRATION,
            "video/mp4",
            "source",
            "put",
            NOW + 5,
        ),
    )
    connection.execute(
        SQL["video_metadata_update"],
        (VIDEO, ACTOR, 12.5, 12_500, 1920.0, 1080.0, 60.0, NOW + 5),
    )
    result = '{"presignedPutData":{"type":"put"}}'
    connection.execute(
        SQL["operation_complete"], (operation, digest("signed"), result, NOW + 5)
    )
    intent = connection.execute(
        "SELECT state,content_type FROM legacy_core_storage_object_intents_v1 WHERE object_key=?",
        (key,),
    ).fetchone()
    assert tuple(intent) == ("capability_issued", "video/mp4")
    second = uid(432)
    insert_operation(
        connection,
        second,
        "cap-v1-7f87205cb7d39ee6",
        "signed",
        "signed-2",
    )
    connection.execute(
        SQL["object_intent_insert"],
        (
            uid(433),
            key,
            second,
            ACTOR,
            ORG,
            VIDEO,
            LEGACY_VIDEO,
            INTEGRATION,
            "video/mp4",
            "source",
            "put",
            NOW + 6,
        ),
    )
    assert connection.execute(
        "SELECT COUNT(*) FROM legacy_core_storage_object_intents_v1 WHERE object_key=?",
        (key,),
    ).fetchone()[0] == 2
    try:
        connection.execute(
            SQL["object_intent_insert"],
            (
                uid(434),
                key,
                second,
                ACTOR,
                ORG,
                VIDEO,
                LEGACY_VIDEO,
                INTEGRATION,
                "video/mp4",
                "source",
                "put",
                NOW + 6,
            ),
        )
        raise AssertionError("one operation created duplicate capability evidence")
    except sqlite3.IntegrityError as error:
        assert "UNIQUE constraint failed" in str(error)
    metadata = connection.execute(
        "SELECT legacy_duration_seconds,duration_ms,legacy_storage_width,legacy_storage_height,legacy_storage_fps FROM videos WHERE id=?",
        (VIDEO,),
    ).fetchone()
    assert tuple(metadata) == (12.5, 12_500, 1920.0, 1080.0, 60.0)


def prove_recording_finalize_intent_is_provider_gated(connection: sqlite3.Connection) -> None:
    operation = uid(440)
    insert_operation(
        connection,
        operation,
        "cap-v1-f9deb8104204a30d",
        "recording_complete",
        "recording-complete",
    )
    connection.execute(
        SQL["finalize_intent_insert"],
        (VIDEO, LEGACY_VIDEO, operation, ACTOR, ORG, NOW + 7),
    )
    connection.execute(
        SQL["operation_effect_pending"],
        (
            operation,
            digest("recording-complete"),
            '{"providerGate":"provider_execution","workflow":"recording-complete"}',
        ),
    )
    intent = connection.execute(
        SQL["finalize_intent_select"],
        (VIDEO, LEGACY_VIDEO, ACTOR, ORG),
    ).fetchone()
    assert tuple(intent) == (operation, "provider_pending")
    assert connection.execute(
        "SELECT state FROM legacy_core_storage_operations_v1 WHERE operation_id=?", (operation,)
    ).fetchone()[0] == "effect_pending"
    try:
        connection.execute(
            "DELETE FROM legacy_core_storage_finalize_intents_v1 WHERE mapped_video_id=?",
            (VIDEO,),
        )
        raise AssertionError("provider intent was deletable")
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_core_storage_finalize_immutable_v1" in str(error)


def main() -> int:
    connection = database()
    seed(connection)
    prove_authority_and_historical_prefix(connection)
    prove_initiate_and_replay_are_immutable(connection)
    prove_completion_stops_at_provider_gate(connection)
    prove_abort_two_phase_terminal_replay(connection)
    prove_signed_capability_is_no_overwrite_governed(connection)
    prove_recording_finalize_intent_is_provider_gated(connection)
    assert connection.execute("PRAGMA foreign_key_check").fetchall() == []
    print(
        "legacy core storage SQLite conformance passed: tenant non-disclosure, historical prefixes, "
        "immutable replay, multipart two-phase state, no-overwrite governance, and provider gates"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
