#!/usr/bin/env python3
"""Provider-free SQLite proof for the ten legacy video-property operations."""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import sqlite3
import uuid
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_video_properties"
MIGRATION = MIGRATIONS / "0044_legacy_video_properties_expand.sql"
RUNTIME = ROOT / "apps/control-plane/src/legacy_video_properties_runtime.rs"
WEB_RUNTIME = ROOT / "apps/control-plane/src/legacy_video_properties_web_runtime.rs"
APPLICATION = ROOT / "crates/application/src/legacy_video_properties.rs"
NOW = 1_735_787_045_006

OPERATIONS = {
    "mobile_password": "cap-v1-2cfe7fc40a6f5a78",
    "mobile_sharing": "cap-v1-5fdf332d1448aedc",
    "mobile_title": "cap-v1-b2db0e7ec51f7898",
    "metadata_put": "cap-v1-5b36dac105856ede",
    "edit_date": "cap-v1-96c52e9330f9a131",
    "edit_title": "cap-v1-6e9f3d370f1ce239",
    "remove_password": "cap-v1-ab11637faa2de45e",
    "set_password": "cap-v1-455e6a1b82e647d9",
    "verify_password": "cap-v1-0a2c44d7a626a1fe",
    "update_settings": "cap-v1-49dba3fbc7c4a74c",
}

SQL = {
    path.stem: path.read_text(encoding="utf-8").strip()
    for path in sorted(QUERIES.glob("*.sql"))
}


def identifier(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


OWNER = identifier(1)
OTHER = identifier(2)
ORG = identifier(10)
VIDEO = identifier(20)
SPACE_A = identifier(30)
SPACE_B = identifier(31)


def digest(label: str) -> str:
    return hashlib.sha256(label.encode()).hexdigest()


def result_digest(kind: str, payload: str) -> str:
    hasher = hashlib.sha256(b"frame.legacy-video-property.result.v1\0")
    for field in (kind, payload):
        encoded = field.encode()
        hasher.update(len(encoded).to_bytes(8, "big"))
        hasher.update(encoded)
    return hasher.hexdigest()


def password_hash(password: str, salt_byte: int) -> str:
    salt = bytes([salt_byte]) * 16
    derived = hashlib.pbkdf2_hmac("sha256", password.encode(), salt, 100_000, 32)
    return base64.b64encode(salt + derived).decode()


def migrated_database() -> sqlite3.Connection:
    database = sqlite3.connect(":memory:", isolation_level=None)
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        database.executescript(migration.read_text(encoding="utf-8"))
        assert not database.execute("PRAGMA foreign_key_check").fetchall(), migration
    return database


def seed(database: sqlite3.Connection) -> None:
    for user_id, name in ((OWNER, "owner"), (OTHER, "other")):
        database.execute(
            """INSERT INTO users(
                 id,email,display_name,created_at_ms,updated_at_ms,status,
                 active_organization_id
               ) VALUES (?,?,?,?,?,'active',?)""",
            (user_id, f"{name}@example.test", name, NOW, NOW, ORG),
        )
    database.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
           ) VALUES (?,?,?,'active','{}',?,?)""",
        (ORG, OWNER, "Frame", NOW, NOW),
    )
    database.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES (?,?,'owner','active',1,?,?)""",
        (ORG, OWNER, NOW, NOW),
    )
    database.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id,
             privacy
           ) VALUES (?,?,?,'ready',?,?,?,'private')""",
        (VIDEO, OWNER, "Original", NOW, NOW, ORG),
    )
    for space_id, name in ((SPACE_A, "A"), (SPACE_B, "B")):
        database.execute(
            """INSERT INTO spaces(
                 id,organization_id,created_by_user_id,name,created_at_ms,updated_at_ms
               ) VALUES (?,?,?,?,?,?)""",
            (space_id, ORG, OWNER, name, NOW, NOW),
        )
        database.execute(
            """INSERT INTO space_videos(
                 space_id,video_id,added_by_user_id,added_at_ms
               ) VALUES (?,?,?,?)""",
            (space_id, VIDEO, OWNER, NOW),
        )


def snapshot(database: sqlite3.Connection) -> sqlite3.Row:
    rows = database.execute(SQL["video_snapshot"], (VIDEO, VIDEO)).fetchall()
    assert len(rows) == 1
    return rows[0]


def operation_id(number: int) -> str:
    return identifier(100 + number)


def claim_and_assert(
    database: sqlite3.Connection,
    operation: str,
    kind: str,
    principal: str,
    request: str,
    key: str,
) -> None:
    source = OPERATIONS[kind]
    legacy_digest = digest(f"legacy:{VIDEO}")
    database.execute(
        SQL["operation_claim"],
        (
            operation,
            source,
            kind,
            principal,
            VIDEO,
            legacy_digest,
            key,
            request,
            NOW,
        ),
    )
    database.execute(
        SQL["operation_assert"],
        (
            operation,
            source,
            kind,
            principal,
            VIDEO,
            legacy_digest,
            key,
            request,
        ),
    )


def assert_owner(database: sqlite3.Connection, operation: str, row: sqlite3.Row) -> None:
    database.execute(
        SQL["owner_assert"],
        (
            operation,
            VIDEO,
            OWNER,
            row["title"],
            row["metadata_json"],
            row["legacy_public"],
            row["password_hash"],
            row["settings_json"],
            row["revision"],
            row["property_revision"],
            row["updated_at_ms"],
        ),
    )


def evidence(
    database: sqlite3.Connection,
    operation: str,
    kind: str,
    principal: str,
    request: str,
    result_kind: str,
    payload: str,
    effect: tuple[str, str] | None,
    now: int,
) -> None:
    receipt_digest = result_digest(result_kind, payload)
    database.execute(
        SQL["receipt_insert"],
        (operation, result_kind, payload, receipt_digest, now),
    )
    if effect:
        database.execute(SQL["effect_insert"], (operation, *effect, now))
    database.execute(
        SQL["audit_insert"],
        (
            str(uuid.uuid4()),
            operation,
            OPERATIONS[kind],
            principal,
            digest(f"video:{VIDEO}"),
            request,
            receipt_digest,
            now,
        ),
    )
    database.execute(SQL["operation_complete"], (operation, now))
    database.execute(
        SQL["durable_assert"],
        (operation, now, receipt_digest, int(effect is not None)),
    )
    database.execute(SQL["assertion_cleanup"], (operation,))


def mutate(
    database: sqlite3.Connection,
    *,
    number: int,
    kind: str,
    title: str | None = None,
    metadata: Any = ...,
    public: int | None = None,
    password: str | None | object = ...,
    settings: Any = ...,
    result_kind: str = "success",
    payload: str = '{"success":true}',
    effect: tuple[str, str] | None = None,
) -> None:
    row = snapshot(database)
    operation = operation_id(number)
    principal = digest("principal:owner")
    request = digest(f"request:{kind}:{number}")
    key = digest(f"key:{kind}:{number}")
    next_title = row["title"] if title is None else title
    next_metadata = (
        row["metadata_json"]
        if metadata is ...
        else json.dumps(metadata, separators=(",", ":"))
    )
    next_public = row["legacy_public"] if public is None else public
    next_password = row["password_hash"] if password is ... else password
    next_settings = (
        row["settings_json"]
        if settings is ...
        else json.dumps(settings, separators=(",", ":"))
    )
    now = NOW + number
    database.execute("BEGIN")
    try:
        claim_and_assert(database, operation, kind, principal, request, key)
        assert_owner(database, operation, row)
        database.execute(
            SQL["mutation_apply"],
            (
                VIDEO,
                int(title is not None),
                next_title,
                int(metadata is not ...),
                next_metadata,
                int(public is not None),
                next_public,
                int(password is not ...),
                next_password,
                int(settings is not ...),
                next_settings,
                now,
                operation,
                row["revision"],
                row["property_revision"],
            ),
        )
        database.execute(
            SQL["mutation_assert"],
            (
                operation,
                VIDEO,
                next_title,
                next_metadata,
                next_public,
                next_password,
                next_settings,
                now,
                row["revision"] + 1,
                row["property_revision"] + 1,
            ),
        )
        evidence(
            database,
            operation,
            kind,
            principal,
            request,
            result_kind,
            payload,
            effect,
            now,
        )
        database.execute("COMMIT")
    except BaseException:
        database.execute("ROLLBACK")
        raise


def verify(database: sqlite3.Connection, number: int, password: str) -> str | None:
    row = snapshot(database)
    candidates = database.execute(SQL["verification_candidates"], (VIDEO,)).fetchall()
    matched = None
    for candidate in candidates:
        wire = base64.b64decode(candidate["password_hash"])
        actual = hashlib.pbkdf2_hmac("sha256", password.encode(), wire[:16], 100_000, 32)
        if actual == wire[16:]:
            matched = candidate["password_hash"]
            break
    operation = operation_id(number)
    principal = digest("principal:anonymous")
    request = digest(f"request:verify:{number}")
    key = digest(f"key:verify:{number}")
    payload = (
        json.dumps({"matchedHash": matched}, separators=(",", ":"))
        if matched
        else '{"success":false}'
    )
    result_kind = "password_verified" if matched else "password_rejected"
    effect = (
        ("password_cookie", payload)
        if matched
        else None
    )
    database.execute("BEGIN")
    try:
        claim_and_assert(database, operation, "verify_password", principal, request, key)
        database.execute(
            SQL["verification_assert"],
            (
                operation,
                VIDEO,
                row["password_hash"],
                row["property_revision"],
                row["joined_space_count"],
                row["joined_space_snapshot"],
            ),
        )
        evidence(
            database,
            operation,
            "verify_password",
            principal,
            request,
            result_kind,
            payload,
            effect,
            NOW + number,
        )
        database.execute("COMMIT")
    except BaseException:
        database.execute("ROLLBACK")
        raise
    return matched


def test_static_contract() -> None:
    database = migrated_database()
    video_columns = {row["name"] for row in database.execute("PRAGMA table_info(videos)")}
    assert {
        "legacy_public",
        "legacy_password_hash",
        "legacy_settings_json",
        "legacy_metadata_json",
        "legacy_property_revision",
        "legacy_property_last_operation_id",
    } <= video_columns
    assert len(SQL) == 16
    texts = "\n".join(
        path.read_text(encoding="utf-8")
        for path in (MIGRATION, RUNTIME, WEB_RUNTIME, APPLICATION)
    )
    for operation in OPERATIONS.values():
        assert operation in texts
    for token in (
        "pbkdf2_hmac::<Sha256>",
        "100_000",
        "x-cap-password",
        "MAX_VERIFIED_HASHES",
        "javascript_object_spread",
        "normalize_playback_speed",
    ):
        assert token in texts


def test_all_mutations_replay_and_password_order() -> None:
    database = migrated_database()
    seed(database)
    paths = json.dumps(
        {"paths": ["/dashboard/caps", "/dashboard/shared-caps", f"/s/{VIDEO}"]},
        separators=(",", ":"),
    )
    mutate(database, number=1, kind="mobile_sharing", public=1, result_kind="mobile_summary", payload='{"id":"video"}')
    mutate(database, number=2, kind="mobile_title", title="Trimmed", result_kind="mobile_summary", payload='{"id":"video"}', effect=("revalidation", paths))
    mutate(database, number=3, kind="metadata_put", metadata=["truthy-array"], result_kind="json_true", payload="true")
    mutate(database, number=4, kind="edit_date", metadata={"0": "truthy-array", "customCreatedAt": "2025-01-02T03:04:05.006Z"}, effect=("revalidation", json.dumps({"paths": ["/dashboard/caps", "/dashboard/shared-caps"]}, separators=(",", ":"))))
    mutate(database, number=5, kind="edit_title", title="   ", metadata={"0": "truthy-array", "customCreatedAt": "2025-01-02T03:04:05.006Z", "titleManuallyEdited": True}, effect=("revalidation", paths))
    hashed = password_hash("  browser  ", 7)
    mutate(database, number=6, kind="set_password", password=hashed, result_kind="password_set", effect=("revalidation", paths))
    mutate(database, number=7, kind="update_settings", settings={"unknown": {"kept": True}, "defaultPlaybackSpeed": 1.0})
    assert verify(database, 8, "  browser  ") == hashed

    space_hash = password_hash("space-secret", 8)
    database.execute(
        "UPDATE spaces SET legacy_password_hash=?, legacy_password_revision=1 WHERE id=?",
        (space_hash, SPACE_A),
    )
    assert verify(database, 9, "space-secret") == space_hash
    assert verify(database, 10, "wrong") is None
    mutate(database, number=11, kind="remove_password", password=None, result_kind="password_removed", effect=("revalidation", paths))
    mutate(database, number=12, kind="mobile_password", password=None, result_kind="mobile_summary", payload='{"id":"video"}')

    rows = database.execute(
        SQL["operation_by_key"],
        (
            OPERATIONS["set_password"],
            digest("principal:owner"),
            VIDEO,
            digest("key:set_password:6"),
        ),
    ).fetchall()
    assert len(rows) == 1 and rows[0]["state"] == "complete"
    assert rows[0]["result_kind"] == "password_set"
    assert rows[0]["audit_count"] == 1


def test_stale_snapshot_rolls_back_and_evidence_is_immutable() -> None:
    database = migrated_database()
    seed(database)
    row = snapshot(database)
    operation = operation_id(50)
    principal = digest("principal:owner")
    request = digest("request:stale")
    key = digest("key:stale")
    database.execute(
        "UPDATE videos SET updated_at_ms=updated_at_ms+1, legacy_property_revision=legacy_property_revision+1 WHERE id=?",
        (VIDEO,),
    )
    database.execute("BEGIN")
    try:
        claim_and_assert(database, operation, "mobile_title", principal, request, key)
        assert_owner(database, operation, row)
        raise AssertionError("stale owner snapshot accepted")
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_video_property_assertion_v1" in str(error)
        database.execute("ROLLBACK")
    assert not database.execute(
        "SELECT 1 FROM legacy_video_property_operations_v1 WHERE operation_id=?",
        (operation,),
    ).fetchall()

    mutate(database, number=51, kind="metadata_put", metadata={"safe": True}, result_kind="json_true", payload="true")
    immutable = operation_id(51)
    for statement in (
        "UPDATE legacy_video_property_receipts_v1 SET result_json='false' WHERE operation_id=?",
        "DELETE FROM legacy_video_property_audit_v1 WHERE operation_id=?",
        "DELETE FROM legacy_video_property_operations_v1 WHERE operation_id=?",
    ):
        try:
            database.execute(statement, (immutable,))
            raise AssertionError("immutable evidence changed")
        except sqlite3.IntegrityError as error:
            assert "frame_legacy_video_property_evidence_immutable_v1" in str(error)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", "--evidence-out", dest="evidence", type=Path)
    args = parser.parse_args()
    tests = (
        test_static_contract,
        test_all_mutations_replay_and_password_order,
        test_stale_snapshot_rolls_back_and_evidence_is_immutable,
    )
    for test in tests:
        test()
        print(f"PASS {test.__name__}")
    evidence = {
        "schema_version": 1,
        "family": "legacy_video_properties.v1",
        "migration": MIGRATION.name,
        "migration_sha256": hashlib.sha256(MIGRATION.read_bytes()).hexdigest(),
        "operation_count": len(OPERATIONS),
        "query_count": len(SQL),
        "query_surface_sha256": hashlib.sha256(
            "".join(
                f"{path.name}\0{hashlib.sha256(path.read_bytes()).hexdigest()}\n"
                for path in sorted(QUERIES.glob("*.sql"))
            ).encode()
        ).hexdigest(),
        "tests": [test.__name__ for test in tests],
        "passed": len(tests),
        "plaintext_passwords": False,
        "native_metadata_isolation": "legacy_metadata_json",
    }
    if args.evidence is not None:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    print(json.dumps(evidence, sort_keys=True))


if __name__ == "__main__":
    main()
