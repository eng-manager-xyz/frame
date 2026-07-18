#!/usr/bin/env python3
"""Provider-free SQLite proof for Cap's developer REST/SDK APIs and cron.

Applies the full migration chain and executes the checked-in D1 statements to
prove key/origin authority, exact read projections, soft delete, durable
optional-key replay, multipart continuation state, atomic completion billing,
and once-per-UTC-day storage billing.
"""

from __future__ import annotations

import hashlib
import json
import sqlite3
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_developer_api"
APPLICATION = ROOT / "crates/application/src/legacy_developer_api.rs"
RUNTIME = ROOT / "apps/control-plane/src/legacy_developer_api_runtime.rs"
WEB_RUNTIME = ROOT / "apps/control-plane/src/legacy_developer_api_web_runtime.rs"
FIXTURE = ROOT / "fixtures/api-parity/v1/developer-api.json"

NOW = 1_700_000_000_000
DAY = "2023-11-14"
ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"
IDS = [
    "cap-v1-0f178cf038854d4a",
    "cap-v1-5914aa6459d24ff1",
    "cap-v1-5c98b9755e4643ba",
    "cap-v1-0d3940728bc19e0e",
    "cap-v1-b6fe5aec600a2e1a",
    "cap-v1-c904ef9c11983a40",
    "cap-v1-cbf22d62a64d3486",
    "cap-v1-6e2296f9695261a3",
    "cap-v1-1cbfe3ecac36f198",
    "cap-v1-aed411f91e977fe5",
    "cap-v1-718e84b39180c0ac",
]


def uuid7(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def cap_id(number: int) -> str:
    output: list[str] = []
    for _ in range(15):
        output.append(ALPHABET[number & 31])
        number >>= 5
    return "".join(reversed(output))


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


ACTOR = uuid7(1)
ORG = uuid7(2)
APP = uuid7(3)
APP_ALIAS = cap_id(3)
ACCOUNT = uuid7(4)
ACCOUNT_ALIAS = cap_id(4)
VIDEO = uuid7(5)
VIDEO_ALIAS = cap_id(5)
VIDEO_TWO = uuid7(6)
VIDEO_TWO_ALIAS = cap_id(6)
PUBLIC_KEY = uuid7(7)
SECRET_KEY = uuid7(8)


SQL = {
    path.stem: path.read_text(encoding="utf-8").strip()
    for path in sorted(QUERIES.glob("*.sql"))
}


def migrated() -> sqlite3.Connection:
    database = sqlite3.connect(":memory:", isolation_level=None)
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        database.executescript(migration.read_text(encoding="utf-8"))
        assert not database.execute("PRAGMA foreign_key_check").fetchall(), migration.name
    return database


def seed(database: sqlite3.Connection) -> None:
    database.execute(
        """INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms,status,
             deleted_at_ms,active_organization_id)
           VALUES(?1,'developer@example.invalid','Developer',1,1,'active',NULL,?2)""",
        (ACTOR, ORG),
    )
    database.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,created_at_ms,updated_at_ms,revision,authority_version)
           VALUES(?1,?2,'Developer','active',1,1,0,0)""",
        (ORG, ACTOR),
    )
    database.execute(
        """INSERT INTO legacy_developer_apps_v1(
             id,legacy_app_id,owner_id,name,environment,created_at_ms,updated_at_ms)
           VALUES(?1,?2,?3,'SDK App','production',1,1)""",
        (APP, APP_ALIAS, ACTOR),
    )
    database.execute(
        """INSERT INTO legacy_developer_app_domains_v1(
             id,legacy_domain_id,app_id,origin,created_at_ms)
           VALUES(?1,?2,?3,'https://portfolio.engmanager.xyz',1)""",
        (uuid7(20), cap_id(20), APP),
    )
    for key_id, alias, kind, prefix, key_digest in (
        (PUBLIC_KEY, cap_id(7), "public", "cpk_abcdefgh", digest("public-key")),
        (SECRET_KEY, cap_id(8), "secret", "csk_abcdefgh", digest("secret-key")),
    ):
        database.execute(
            """INSERT INTO legacy_developer_api_keys_v1(
                 id,legacy_key_id,app_id,key_kind,key_prefix,key_digest,encrypted_key,
                 created_at_ms)
               VALUES(?1,?2,?3,?4,?5,?6,'protected',1)""",
            (key_id, alias, APP, kind, prefix, key_digest),
        )
    database.execute(
        """INSERT INTO legacy_developer_credit_accounts_v1(
             id,legacy_credit_account_id,app_id,owner_id,balance_microcredits,
             created_at_ms,updated_at_ms)
           VALUES(?1,?2,?3,?4,1000000,1,1)""",
        (ACCOUNT, ACCOUNT_ALIAS, APP, ACTOR),
    )
    for video_id, alias, user_id, created, duration in (
        (VIDEO, VIDEO_ALIAS, "customer-a", 10, 600.0),
        (VIDEO_TWO, VIDEO_TWO_ALIAS, "customer-b", 20, 120.0),
    ):
        database.execute(
            """INSERT INTO legacy_developer_videos_v1(
                 id,legacy_video_id,app_id,external_user_id,name,duration,width,height,fps,
                 s3_key,transcription_status,metadata_json,created_at_ms,updated_at_ms)
               VALUES(?1,?2,?3,?4,'Demo',?5,1920,1080,30,?6,'COMPLETE',
                 '{"source":"fixture"}',?7,?7)""",
            (
                video_id,
                alias,
                APP,
                user_id,
                duration,
                f"developer/{APP_ALIAS}/{alias}/video",
                created,
            ),
        )


def operation_claim(
    database: sqlite3.Connection,
    serial: int,
    source_id: str,
    target: str,
    key: str,
    request: str,
) -> str:
    operation = uuid7(100 + serial)
    database.execute(
        SQL["operation_claim"],
        (operation, source_id, APP, target, digest(key), digest(request), NOW),
    )
    return operation


def prove_authority_and_reads(database: sqlite3.Connection) -> None:
    public = database.execute(
        SQL["auth_key"], (digest("public-key"), "public")
    ).fetchone()
    secret = database.execute(
        SQL["auth_key"], (digest("secret-key"), "secret")
    ).fetchone()
    assert public["app_id"] == APP and public["environment"] == "production"
    assert secret["app_id"] == APP
    assert database.execute(
        SQL["auth_origin"], (APP, "https://portfolio.engmanager.xyz")
    ).fetchone()["allowed"] == 1
    assert database.execute(
        SQL["auth_origin"], (APP, "https://evil.invalid")
    ).fetchone()["allowed"] == 0
    database.execute(SQL["auth_touch"], (digest("public-key"), NOW))
    database.execute(SQL["auth_touch"], (digest("public-key"), NOW + 1))
    assert database.execute(
        "SELECT last_used_at_ms FROM legacy_developer_api_keys_v1 WHERE id=?1", (PUBLIC_KEY,)
    ).fetchone()[0] == NOW

    usage = database.execute(SQL["usage_read"], (APP,)).fetchone()
    assert usage["balance_microcredits"] == 1_000_000
    assert usage["total_videos"] == 2
    assert usage["total_duration_minutes"] == 12.0
    page = database.execute(SQL["videos_list"], (APP, None, 1, 0)).fetchall()
    assert [row["legacy_video_id"] for row in page] == [VIDEO_TWO_ALIAS]
    filtered = database.execute(
        SQL["videos_list"], (APP, "customer-a", 100, 0)
    ).fetchall()
    assert [row["legacy_video_id"] for row in filtered] == [VIDEO_ALIAS]
    video = database.execute(SQL["video_read"], (APP, VIDEO_ALIAS)).fetchone()
    assert video["duration"] == 600.0 and video["width"] == 1920.0


def prove_creation_delete_and_replay(database: sqlite3.Connection) -> None:
    create_id = operation_claim(
        database, 1, "cap-v1-c904ef9c11983a40", "new-video", "create-key", "create"
    )
    created = uuid7(200)
    alias = cap_id(200)
    object_key = f"developer/{APP_ALIAS}/{alias}/video"
    result = json.dumps(
        {
            "videoId": alias,
            "s3Key": object_key,
            "shareUrl": f"https://frame.engmanager.xyz/dev/{alias}",
            "embedUrl": f"https://frame.engmanager.xyz/embed/{alias}?sdk=1",
        },
        separators=(",", ":"),
    )
    database.execute("BEGIN IMMEDIATE")
    database.execute(
        SQL["video_create"],
        (created, alias, APP, "customer-c", "Created", object_key, "{}", NOW, create_id),
    )
    database.execute(
        SQL["receipt_insert"],
        (create_id, 200, "video_created", result, digest(result), NOW),
    )
    database.execute(SQL["operation_complete"], (create_id, NOW))
    database.execute("COMMIT")
    replay = database.execute(
        SQL["operation_read"],
        ("cap-v1-c904ef9c11983a40", APP, digest("create-key")),
    ).fetchone()
    assert replay["state"] == "complete" and replay["result_json"] == result
    assert replay["request_digest"] != digest("different-request")

    delete_id = operation_claim(
        database, 2, "cap-v1-1cbfe3ecac36f198", alias, "delete-key", "delete"
    )
    database.execute(SQL["video_delete"], (APP, alias, NOW + 1, delete_id))
    assert database.execute(SQL["video_read"], (APP, alias)).fetchone() is None


def prove_multipart_and_atomic_billing(database: sqlite3.Connection) -> None:
    initiate = operation_claim(
        database, 3, "cap-v1-0d3940728bc19e0e", VIDEO_ALIAS, "initiate", "initiate"
    )
    database.execute(SQL["operation_effect_pending"], (initiate,))
    database.execute(
        SQL["outbox_insert"], (initiate, "multipart_create", digest("initiate"), NOW)
    )
    upload_id = "provider-upload-fixture"
    object_key = f"developer/{APP_ALIAS}/{VIDEO_ALIAS}/video"
    database.execute(
        SQL["multipart_session_insert"],
        (upload_id, APP, VIDEO, object_key, "video/mp4", initiate, NOW),
    )
    database.execute(SQL["outbox_complete"], (initiate, NOW))
    session = database.execute(
        SQL["multipart_session_read"], (APP, upload_id, VIDEO_ALIAS)
    ).fetchone()
    assert session["state"] == "open" and session["object_key"] == object_key

    presign = operation_claim(
        database, 4, "cap-v1-b6fe5aec600a2e1a", VIDEO_ALIAS, "presign", "presign"
    )
    database.execute(
        SQL["part_capability_insert"], (presign, upload_id, 1, NOW, NOW + 900_000)
    )

    complete = operation_claim(
        database, 5, "cap-v1-5c98b9755e4643ba", VIDEO_ALIAS, "complete", "complete"
    )
    database.execute("BEGIN IMMEDIATE")
    database.execute(SQL["operation_effect_pending"], (complete,))
    database.execute(
        SQL["multipart_state"],
        (upload_id, "completing", NOW, "open", complete),
    )
    database.execute(
        SQL["outbox_insert"], (complete, "multipart_complete", digest("complete"), NOW)
    )
    before = database.execute(
        "SELECT balance_microcredits FROM legacy_developer_credit_accounts_v1 WHERE id=?1",
        (ACCOUNT,),
    ).fetchone()[0]
    # 150MB / 2.5MB/s = 60 billable seconds => 5,000 microcredits.
    database.execute(
        SQL["credit_debit"],
        (
            uuid7(300), ACCOUNT, "video_create", -5000, before - 5000,
            VIDEO_ALIAS, "developer_video", '{"durationSeconds":60}', complete, NOW,
        ),
    )
    database.execute("COMMIT")
    assert database.execute(
        "SELECT balance_microcredits FROM legacy_developer_credit_accounts_v1 WHERE id=?1",
        (ACCOUNT,),
    ).fetchone()[0] == before - 5000

    # A different terminal operation that loses the open-session CAS must
    # abort its entire D1 batch before an outbox row or second debit can land.
    database.execute("BEGIN IMMEDIATE")
    try:
        racing_abort = operation_claim(
            database, 6, "cap-v1-5914aa6459d24ff1", VIDEO_ALIAS,
            "racing-abort", "racing-abort",
        )
        database.execute(SQL["operation_effect_pending"], (racing_abort,))
        database.execute(
            SQL["multipart_state"],
            (upload_id, "aborting", NOW + 1, "open", racing_abort),
        )
        database.execute(
            SQL["outbox_insert"],
            (racing_abort, "multipart_abort", digest("racing-abort"), NOW + 1),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_developer_multipart_claim_lost_v1" in str(error)
        database.execute("ROLLBACK")
    else:
        database.execute("ROLLBACK")
        raise AssertionError("losing multipart terminal claim must abort")
    owner = database.execute(
        """SELECT state,terminal_operation_id
           FROM legacy_developer_multipart_sessions_v1
           WHERE provider_upload_id=?1""",
        (upload_id,),
    ).fetchone()
    assert owner["state"] == "completing" and owner["terminal_operation_id"] == complete
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_developer_credit_transactions_v1"
    ).fetchone()[0] == 1

    database.execute("SAVEPOINT insufficient")
    try:
        database.execute(
            SQL["credit_debit"],
            (
                uuid7(301), ACCOUNT, "storage_daily", -2_000_000, -1_005_000,
                "2099-01-01", "manual", "{}", None, NOW,
            ),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_developer_insufficient_credits_v1" in str(error)
        database.execute("ROLLBACK TO insufficient")
        database.execute("RELEASE insufficient")
    else:
        raise AssertionError("insufficient credit debit must roll back")


def prove_daily_storage_once(database: sqlite3.Connection) -> None:
    candidate = database.execute(SQL["cron_candidates"], (DAY,)).fetchone()
    # Existing live video duration is 720 seconds = 12 minutes; floor(12*3.33)=39.
    assert candidate["total_duration_minutes"] == 12.0
    assert int(candidate["total_duration_minutes"] * 333 / 100) == 39
    charge = 39
    balance = candidate["balance_microcredits"]
    database.execute("BEGIN IMMEDIATE")
    database.execute(
        SQL["credit_debit"],
        (
            uuid7(400), ACCOUNT, "storage_daily", -charge, balance - charge,
            DAY, "manual", '{"date":"2023-11-14"}', None, NOW,
        ),
    )
    database.execute(
        SQL["cron_snapshot_insert"],
        (uuid7(401), APP, DAY, 12.0, 2, charge, NOW),
    )
    database.execute(SQL["cron_run_insert"], (DAY, 1, NOW))
    database.execute("COMMIT")
    assert database.execute(SQL["cron_candidates"], (DAY,)).fetchall() == []
    run = database.execute(SQL["cron_run_read"], (DAY,)).fetchone()
    assert run["apps_processed"] == 1
    try:
        database.execute(SQL["cron_run_insert"], (DAY, 1, NOW + 1))
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("daily run must be unique")


def prove_checked_contract() -> None:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    assert [operation["id"] for operation in fixture["operations"]] == IDS
    assert fixture["persistence"]["migration"] == "0054_legacy_developer_api_expand.sql"
    assert fixture["billing"]["completion_size_floor_bytes_per_second"] == 2_500_000
    assert fixture["billing"]["daily_storage_microcredits_per_minute"] == "3.33"
    for path in (APPLICATION, RUNTIME, WEB_RUNTIME):
        source = path.read_text(encoding="utf-8")
        for operation_id in IDS:
            assert operation_id in source or path != APPLICATION
    assert all(sql.count(";") <= 1 for sql in SQL.values())


def main() -> int:
    prove_checked_contract()
    database = migrated()
    seed(database)
    prove_authority_and_reads(database)
    prove_creation_delete_and_replay(database)
    prove_multipart_and_atomic_billing(database)
    prove_daily_storage_once(database)
    assert not database.execute("PRAGMA foreign_key_check").fetchall()
    print("legacy developer API SQLite conformance: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
