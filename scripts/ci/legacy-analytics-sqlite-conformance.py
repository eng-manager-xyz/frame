#!/usr/bin/env python3
"""Provider-free SQLite proof for Cap's seven retained analytics operations.

The proof applies every migration and executes the checked-in D1 statements.
It covers optional-auth video non-disclosure, active-organization dashboard
scope, the provider-free signup CAS, durable provider staging/replay, first-view
effect claims, rate limits, and immutable outbox completion transitions. It
does not invent Tinybird results or external delivery receipts.
"""

from __future__ import annotations

import hashlib
import json
import sqlite3
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_analytics"
APPLICATION = ROOT / "crates/application/src/legacy_analytics.rs"
RUNTIME = ROOT / "apps/control-plane/src/legacy_analytics_runtime.rs"
WEB_RUNTIME = ROOT / "apps/control-plane/src/legacy_analytics_web_runtime.rs"
FIXTURE = ROOT / "fixtures/api-parity/v1/analytics.json"

NOW = 1_773_000_000_000
CUTOFF = 1_772_582_400_000
DAY = 86_400_000
IDS = [
    "cap-v1-c8a43dc80c502b6d",
    "cap-v1-51dc2aa9f19a48cc",
    "cap-v1-9b093898957efebb",
    "cap-v1-be2ea6b474aae7c9",
    "cap-v1-7c47f9a2a9a24ac0",
    "cap-v1-dd88ded400188c1e",
    "cap-v1-9186738740a1ece1",
]
PROVIDER_IDS = [operation_id for operation_id in IDS if operation_id != IDS[5]]


def identifier(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def legacy_id(number: int) -> str:
    alphabet = "0123456789abcdefghjkmnpqrstvwxyz"
    output: list[str] = []
    for _ in range(15):
        output.append(alphabet[number & 31])
        number >>= 5
    return "".join(reversed(output))


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


OWNER = identifier(1)
MEMBER = identifier(2)
OUTSIDER = identifier(3)
YOUNG = identifier(4)
OLD = identifier(5)
ORG = identifier(10)
OTHER_ORG = identifier(11)
SPACE = identifier(20)
CROSS_SPACE = identifier(21)
PUBLIC = identifier(30)
NAKED_PUBLIC = identifier(31)
PRIVATE = identifier(32)
PASSWORD = identifier(33)
CROSS_VIDEO = identifier(34)
PUBLIC_ALIAS = legacy_id(30)
NAKED_PUBLIC_ALIAS = legacy_id(31)
PRIVATE_ALIAS = legacy_id(32)
PASSWORD_ALIAS = legacy_id(33)
CROSS_VIDEO_ALIAS = legacy_id(34)

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
    users = (
        (OWNER, "owner@engmanager.xyz", "Owner", NOW - DAY, ORG),
        (MEMBER, "member@engmanager.xyz", "Member", NOW - DAY, ORG),
        (OUTSIDER, "outsider@else.invalid", "Outsider", NOW - DAY, OTHER_ORG),
        (YOUNG, "young@example.invalid", "Young", NOW - 7 * DAY, None),
        (OLD, "old@example.invalid", "Old", NOW - 7 * DAY - 1, None),
    )
    for user_id, email, name, created_at_ms, active_org in users:
        database.execute(
            """INSERT INTO users(
                 id,email,display_name,created_at_ms,updated_at_ms,status,
                 deleted_at_ms,active_organization_id,preferences_json
               ) VALUES(?1,?2,?3,?4,?4,'active',NULL,?5,'{}')""",
            (user_id, email, name, created_at_ms, active_org),
        )
    for organization_id, owner_id, name in (
        (ORG, OWNER, "Eng Manager"),
        (OTHER_ORG, OUTSIDER, "Other"),
    ):
        database.execute(
            """INSERT INTO organizations(
                 id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
               ) VALUES(?1,?2,?3,'active','{}',1,1)""",
            (organization_id, owner_id, name),
        )
        database.execute(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,has_pro_seat,
                 created_at_ms,updated_at_ms
               ) VALUES(?1,?2,'owner','active',1,1,1)""",
            (organization_id, owner_id),
        )
    database.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?1,?2,'member','active',0,1,1)""",
        (ORG, MEMBER),
    )
    database.execute(
        """INSERT INTO organization_allowed_domains(
             organization_id,domain_ascii,verified_at_ms,created_at_ms
           ) VALUES(?1,'engmanager.xyz',1,1)""",
        (ORG,),
    )
    for space_id, organization_id, creator, name in (
        (SPACE, ORG, OWNER, "Analytics"),
        (CROSS_SPACE, OTHER_ORG, OUTSIDER, "Other analytics"),
    ):
        database.execute(
            """INSERT INTO spaces(
                 id,organization_id,created_by_user_id,name,is_public,
                 created_at_ms,updated_at_ms
               ) VALUES(?1,?2,?3,?4,0,1,1)""",
            (space_id, organization_id, creator, name),
        )
    database.execute(
        """INSERT INTO space_members(
             space_id,user_id,role,state,created_at_ms,updated_at_ms
           ) VALUES(?1,?2,'viewer','active',1,1)""",
        (SPACE, MEMBER),
    )

    videos = (
        (PUBLIC, OWNER, ORG, "Public", 1, None, CUTOFF + 1),
        (NAKED_PUBLIC, OWNER, None, "Public without tenant", 1, None, CUTOFF + 2),
        (PRIVATE, OWNER, ORG, "Private", 0, None, CUTOFF + 3),
        (PASSWORD, OWNER, ORG, "Password", 1, "A" * 64, CUTOFF + 4),
        (CROSS_VIDEO, OUTSIDER, OTHER_ORG, "Other video", 0, None, CUTOFF + 5),
    )
    for video_id, owner_id, organization_id, title, public, password, created in videos:
        database.execute(
            """INSERT INTO videos(
                 id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id,
                 privacy,legacy_public,legacy_password_hash
               ) VALUES(?1,?2,?3,'ready',?4,?5,?6,
                 CASE WHEN ?7=1 THEN 'public' ELSE 'private' END,?7,?8)""",
            (
                video_id,
                owner_id,
                title,
                created,
                NOW - 120_001,
                organization_id,
                public,
                password,
            ),
        )
    for video_id, alias in (
        (PUBLIC, PUBLIC_ALIAS),
        (NAKED_PUBLIC, NAKED_PUBLIC_ALIAS),
        (PRIVATE, PRIVATE_ALIAS),
        (PASSWORD, PASSWORD_ALIAS),
        (CROSS_VIDEO, CROSS_VIDEO_ALIAS),
    ):
        database.execute(
            """INSERT INTO legacy_collaboration_video_aliases_v1(
                 legacy_video_id,mapped_video_id,provenance,created_at_ms
               ) VALUES(?1,?2,'cap_backfill',1)""",
            (alias, video_id),
        )
    database.execute(
        """INSERT INTO space_videos(
             space_id,video_id,added_by_user_id,added_at_ms
           ) VALUES(?1,?2,?3,1)""",
        (SPACE, PRIVATE, OWNER),
    )
    database.execute(
        """INSERT INTO shared_videos(
             id,video_id,organization_id,shared_by_user_id,sharing_mode,shared_at_ms
           ) VALUES(?1,?2,?3,?4,'organization',1)""",
        (identifier(40), PASSWORD, ORG, OWNER),
    )


def can_view(row: sqlite3.Row | None) -> bool:
    if row is None:
        return False
    if row["actor_is_owner"] == 1:
        return True
    password_allowed = row["password_required"] == 0 or row["password_granted"] == 1
    if row["actor_has_organization_share"] == 1 or row["actor_has_space_share"] == 1:
        return password_allowed
    return row["legacy_public"] == 1 and row["email_allowed"] == 1 and password_allowed


def authority(
    database: sqlite3.Connection,
    video_id: str,
    actor_id: str | None,
    grant: str | None = None,
) -> sqlite3.Row | None:
    return database.execute(SQL["video_authority"], (video_id, actor_id, grant, NOW)).fetchone()


def prove_video_authority(database: sqlite3.Connection) -> None:
    assert can_view(authority(database, PRIVATE_ALIAS, OWNER))
    assert can_view(authority(database, PRIVATE_ALIAS, MEMBER))
    assert not can_view(authority(database, PRIVATE_ALIAS, OUTSIDER))
    assert not can_view(authority(database, PUBLIC_ALIAS, None))
    assert can_view(authority(database, PUBLIC_ALIAS, MEMBER))
    assert not can_view(authority(database, PUBLIC_ALIAS, OUTSIDER))
    assert can_view(authority(database, NAKED_PUBLIC_ALIAS, None))

    password_digest = digest("password-cookie")
    assert not can_view(authority(database, PASSWORD_ALIAS, MEMBER))
    database.execute(
        SQL["password_grant_issue"],
        (
            json.dumps([PASSWORD_ALIAS]),
            json.dumps(["A" * 64]),
            password_digest,
            NOW - 1,
            NOW + DAY,
        ),
    )
    assert can_view(authority(database, PASSWORD_ALIAS, MEMBER, password_digest))
    assert not can_view(authority(database, PASSWORD_ALIAS, MEMBER, digest("wrong")))
    assert can_view(authority(database, PASSWORD_ALIAS, OWNER))
    assert authority(database, "missing-video", MEMBER) is None


def prove_dashboard_authority(database: sqlite3.Connection) -> None:
    scoped = database.execute(
        SQL["dashboard_authority"], (OWNER, ORG, SPACE, PRIVATE_ALIAS)
    ).fetchone()
    assert scoped is not None
    assert scoped["active_organization_id"] == ORG
    assert scoped["organization_allowed"] == 1
    assert scoped["space_allowed"] == 1 and scoped["video_allowed"] == 1
    assert scoped["lifetime_start_ms"] == CUTOFF + 3

    cross_space = database.execute(
        SQL["dashboard_authority"], (OWNER, ORG, CROSS_SPACE, None)
    ).fetchone()
    cross_video = database.execute(
        SQL["dashboard_authority"], (OWNER, ORG, None, CROSS_VIDEO_ALIAS)
    ).fetchone()
    cross_org = database.execute(
        SQL["dashboard_authority"], (OUTSIDER, ORG, None, None)
    ).fetchone()
    assert cross_space["space_allowed"] == 0
    assert cross_video["video_allowed"] == 0
    assert cross_org["active_organization_id"] == OTHER_ORG
    assert cross_org["organization_allowed"] == 0


def prove_signup_cas(database: sqlite3.Connection) -> None:
    first = database.execute(SQL["signup_claim"], (YOUNG, NOW))
    assert first.rowcount == 1
    tracked = database.execute(
        """SELECT json_extract(preferences_json,'$.trackedEvents.user_signed_up')
           FROM users WHERE id=?1""",
        (YOUNG,),
    ).fetchone()[0]
    assert tracked == 1
    assert database.execute(SQL["signup_claim"], (YOUNG, NOW + 1)).rowcount == 0
    assert database.execute(SQL["signup_claim"], (OLD, NOW)).rowcount == 0
    assert database.execute(SQL["signup_claim"], (identifier(999), NOW)).rowcount == 0
    database.execute("UPDATE users SET preferences_json='{}' WHERE id=?1", (YOUNG,))
    assert database.execute(SQL["signup_claim"], (YOUNG, NOW)).rowcount == 1


def claim(
    database: sqlite3.Connection,
    serial: int,
    source_id: str,
    kind: str,
    target: str | None,
    principal: str = "principal",
) -> tuple[str, str, str]:
    operation_id = identifier(1000 + serial)
    request_digest = digest(f"request:{serial}")
    execution_digest = digest(f"execution:{serial}")
    database.execute(
        SQL["operation_claim"],
        (
            operation_id,
            source_id,
            kind,
            digest(principal),
            OWNER,
            ORG,
            target,
            execution_digest,
            request_digest,
            NOW + serial,
        ),
    )
    return operation_id, request_digest, execution_digest


def audit(
    database: sqlite3.Connection,
    serial: int,
    operation_id: str,
    source_id: str,
    request_digest: str,
    result: str,
) -> None:
    database.execute(
        SQL["audit_insert"],
        (
            identifier(5000 + serial),
            operation_id,
            source_id,
            digest("principal"),
            digest("target"),
            request_digest,
            result,
            NOW + serial,
        ),
    )


def prove_query_staging_and_replay(database: sqlite3.Connection) -> None:
    operation, request_digest, execution_digest = claim(
        database, 1, IDS[0], "query", PUBLIC
    )
    request_json = json.dumps(
        {
            "requestedVideoId": PUBLIC_ALIAS,
            "nativeVideoId": PUBLIC,
            "rangeDays": 90,
            "aggregateThenRawFallback": True,
            "failurePolicy": "fail_closed",
        },
        separators=(",", ":"),
    )
    database.execute(
        SQL["query_outbox_insert"],
        (operation, "video_count_route", request_json, request_digest, NOW),
    )
    audit(database, 1, operation, IDS[0], request_digest, "provider_query_pending")
    replay = database.execute(
        SQL["operation_read"], (IDS[0], digest("principal"), execution_digest)
    ).fetchone()
    assert replay["operation_id"] == operation and replay["state"] == "pending"
    try:
        database.execute(
            SQL["operation_claim"],
            (
                identifier(1002), IDS[0], "query", digest("principal"), OWNER, ORG,
                PUBLIC, execution_digest, request_digest, NOW + 2,
            ),
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("provider execution replay key must be unique")

    database.execute(
        """UPDATE legacy_analytics_query_outbox_v1
           SET attempt_count=attempt_count+1
           WHERE operation_id=?1""",
        (operation,),
    )
    retry = database.execute(
        """SELECT state,attempt_count,completed_at_ms
           FROM legacy_analytics_query_outbox_v1 WHERE operation_id=?1""",
        (operation,),
    ).fetchone()
    assert tuple(retry) == ("pending", 1, None)
    database.execute(
        """UPDATE legacy_analytics_query_outbox_v1
           SET state='complete',attempt_count=attempt_count+1,completed_at_ms=?2
           WHERE operation_id=?1""",
        (operation, NOW + 10),
    )
    database.execute(
        """UPDATE legacy_analytics_provider_operations_v1
           SET state='complete',completed_at_ms=?2 WHERE operation_id=?1""",
        (operation, NOW + 10),
    )
    try:
        database.execute(
            "UPDATE legacy_analytics_query_outbox_v1 SET request_json='{}' WHERE operation_id=?1",
            (operation,),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_analytics_outbox_immutable_v1" in str(error)
    else:
        raise AssertionError("completed provider query payload must be immutable")


def event_payload(video_id: str, session: str) -> str:
    return json.dumps(
        {
            "timestamp": "2026-03-09T00:00:00.000Z",
            "session_id": session,
            "action": "page_hit",
            "version": "1.0",
            "tenant_id": OWNER,
            "video_id": video_id,
            "pathname": f"/s/{video_id}",
            "country": "US",
            "region": "CA",
            "city": "Los Angeles",
            "browser": "Chrome",
            "device": "desktop",
            "os": "Mac OS",
            "user_id": None,
        },
        separators=(",", ":"),
    )


def insert_event(
    database: sqlite3.Connection,
    operation: str,
    video_id: str,
    session: str,
    request_digest: str,
    serial: int,
) -> None:
    database.execute(
        SQL["event_outbox_insert"],
        (
            operation,
            "2026-03-09T00:00:00.000Z",
            session,
            OWNER,
            video_id,
            f"/s/{video_id}",
            "US",
            "CA",
            "Los Angeles",
            "Chrome",
            "desktop",
            "Mac OS",
            "Mozilla/5.0",
            None,
            event_payload(video_id, session),
            NOW + serial,
        ),
    )
    audit(database, serial, operation, IDS[1], request_digest, "provider_event_pending")


def prove_event_and_effect_staging(database: sqlite3.Connection) -> None:
    operation, request_digest, _ = claim(database, 20, IDS[1], "event", PUBLIC)
    insert_event(database, operation, PUBLIC_ALIAS, "viewer-session", request_digest, 20)

    email_payload = json.dumps(
        {
            "videoId": PUBLIC_ALIAS,
            "videoName": "Public",
            "ownerName": "Owner",
            "recipientEmail": "owner@engmanager.xyz",
            "viewerName": "Anonymous Viewer",
            "isAnonymous": True,
        },
        separators=(",", ":"),
    )
    database.execute(
        SQL["email_outbox_insert"],
        (
            operation,
            PUBLIC,
            OWNER,
            "owner@engmanager.xyz",
            None,
            "Anonymous Viewer",
            1,
            email_payload,
            NOW + 20,
        ),
    )
    session_hash = digest("viewer-session")
    notification_payload = json.dumps(
        {
            "videoId": PUBLIC_ALIAS,
            "anonymousName": "Anonymous Walrus",
            "location": "Los Angeles, US",
        },
        separators=(",", ":"),
    )
    database.execute(
        SQL["notification_outbox_insert"],
        (
            operation,
            f"anon_view:{PUBLIC_ALIAS}:{session_hash}",
            PUBLIC,
            ORG,
            OWNER,
            "Anonymous Walrus",
            "Los Angeles, US",
            notification_payload,
            NOW + 20,
        ),
    )
    persisted_email = json.loads(database.execute(
        "SELECT payload_json FROM legacy_analytics_email_outbox_v1 WHERE operation_id=?1",
        (operation,),
    ).fetchone()[0])
    persisted_notification = database.execute(
        """SELECT deduplication_key,payload_json
           FROM legacy_analytics_notification_outbox_v1 WHERE operation_id=?1""",
        (operation,),
    ).fetchone()
    assert persisted_email["videoId"] == PUBLIC_ALIAS
    assert persisted_notification["deduplication_key"] == (
        f"anon_view:{PUBLIC_ALIAS}:{session_hash}"
    )
    assert json.loads(persisted_notification["payload_json"])["videoId"] == PUBLIC_ALIAS
    snapshot = database.execute(SQL["track_video_snapshot"], (PUBLIC,)).fetchone()
    assert snapshot["first_view_email_claimed"] == 1
    assert snapshot["has_active_upload"] == 0

    duplicate, duplicate_digest, _ = claim(database, 21, IDS[1], "event", PUBLIC)
    insert_event(database, duplicate, PUBLIC, "other-session", duplicate_digest, 21)
    assert database.execute(
        SQL["email_outbox_insert"],
        (
            duplicate, PUBLIC, OWNER, "owner@engmanager.xyz", None,
            "Anonymous Viewer", 1, email_payload, NOW + 21,
        ),
    ).rowcount == 0
    assert database.execute(
        SQL["notification_outbox_insert"],
        (
            duplicate, f"anon_view:{PUBLIC_ALIAS}:{session_hash}", PUBLIC, ORG, OWNER,
            "Anonymous Walrus", None, notification_payload, NOW + 21,
        ),
    ).rowcount == 0

    for table in (
        "legacy_analytics_event_outbox_v1",
        "legacy_analytics_email_outbox_v1",
        "legacy_analytics_notification_outbox_v1",
    ):
        database.execute(
            f"""UPDATE {table} SET attempt_count=attempt_count+1
                WHERE operation_id=?1""",
            (operation,),
        )
        retry = database.execute(
            f"""SELECT state,attempt_count,completed_at_ms FROM {table}
                WHERE operation_id=?1""",
            (operation,),
        ).fetchone()
        assert tuple(retry) == ("pending", 1, None)
        database.execute(
            f"""UPDATE {table}
                SET state='complete',attempt_count=attempt_count+1,completed_at_ms=?2
                WHERE operation_id=?1""",
            (operation, NOW + 30),
        )
    database.execute(
        """UPDATE legacy_analytics_provider_operations_v1
           SET state='complete',completed_at_ms=?2 WHERE operation_id=?1""",
        (operation, NOW + 30),
    )
    try:
        database.execute(
            """UPDATE legacy_analytics_email_outbox_v1
               SET recipient_email='changed@example.invalid' WHERE operation_id=?1""",
            (operation,),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_analytics_outbox_immutable_v1" in str(error)
    else:
        raise AssertionError("completed email payload must be immutable")

    missing, missing_digest, _ = claim(database, 22, IDS[1], "event", "missing-video")
    insert_event(database, missing, "missing-video", "missing", missing_digest, 22)
    assert database.execute(SQL["track_video_snapshot"], ("missing-video",)).fetchone() is None

    database.execute(
        """INSERT INTO video_uploads(
             id,organization_id,video_id,state,expected_bytes,received_bytes,
             idempotency_key,source_object_key,source_version,content_type,
             created_at_ms,updated_at_ms,event_sequence,event_fingerprint
           ) VALUES(?1,?2,?3,'initiated',1,0,'analytics-upload',?4,1,
             'video/mp4',1,1,0,
             'daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9')""",
        (identifier(7000), ORG, PRIVATE, f"source/{PRIVATE}"),
    )
    assert database.execute(
        SQL["track_video_snapshot"], (PRIVATE_ALIAS,)
    ).fetchone()["has_active_upload"] == 1


def prove_notification_rate_limit(database: sqlite3.Connection) -> None:
    already = database.execute(
        """SELECT COUNT(*) FROM legacy_analytics_notification_outbox_v1
           WHERE recipient_user_id=?1 AND video_id=?2""",
        (OWNER, PUBLIC),
    ).fetchone()[0]
    assert already == 1
    payload = '{"rate":"fixture"}'
    for offset in range(49):
        operation, _, _ = claim(
            database, 100 + offset, IDS[1], "event", PUBLIC, f"rate-{offset}"
        )
        inserted = database.execute(
            SQL["notification_outbox_insert"],
            (
                operation, f"rate:{offset}", PUBLIC, ORG, OWNER,
                "Anonymous Otter", None, payload, NOW + 100 + offset,
            ),
        )
        assert inserted.rowcount == 1
    blocked, _, _ = claim(database, 200, IDS[1], "event", PUBLIC, "rate-blocked")
    assert database.execute(
        SQL["notification_outbox_insert"],
        (
            blocked, "rate:blocked", PUBLIC, ORG, OWNER,
            "Anonymous Otter", None, payload, NOW + 200,
        ),
    ).rowcount == 0


def prove_checked_contract(database: sqlite3.Connection) -> None:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    assert [operation["id"] for operation in fixture["operations"]] == IDS
    assert [operation["source_count"] for operation in fixture["operations"]] == [5, 6, 5, 5, 8, 3, 4]
    behavior = fixture["completion"]["production_behavior"]
    assert behavior["provider_execution"] == PROVIDER_IDS
    assert behavior["serve_exact_d1"] == [IDS[5]]
    assert fixture["persistence"]["migration"] == "0059_legacy_analytics_expand.sql"
    assert fixture["persistence"]["provider_result_storage"].startswith("absent")
    assert sorted(fixture["persistence"]["queries"]) == sorted(
        f"{path.stem}.sql" for path in QUERIES.glob("*.sql")
    )
    application = APPLICATION.read_text(encoding="utf-8")
    runtime = RUNTIME.read_text(encoding="utf-8")
    web_runtime = WEB_RUNTIME.read_text(encoding="utf-8")
    assert fixture["implementation"]["control_plane_web_module"] == WEB_RUNTIME.stem
    migration = (MIGRATIONS / fixture["persistence"]["migration"]).read_text(encoding="utf-8")
    for operation_id in IDS:
        assert operation_id in application
    assert "LEGACY_ANALYTICS_SIGNUP_OPERATION_ID" in web_runtime
    assert "LEGACY_ANALYTICS_VIDEO_ACTION_OPERATION_ID" in web_runtime
    for carrier in ("http_response", "effect_rpc_response_from_bytes", "action_response"):
        assert f"fn {carrier}" in web_runtime
    for operation_id in PROVIDER_IDS:
        assert operation_id in migration
    for query in fixture["persistence"]["queries"]:
        assert query in runtime or query == "audit_insert.sql"
    assert "ProviderPending" in runtime
    assert "SignupTracking" in application
    assert "count" not in {
        column[1]
        for table in fixture["persistence"]["tables"]
        for column in database.execute(f"PRAGMA table_info({table})").fetchall()
    }
    assert all(statement.count(";") <= 1 for statement in SQL.values())


def main() -> int:
    database = migrated()
    prove_checked_contract(database)
    seed(database)
    prove_video_authority(database)
    prove_dashboard_authority(database)
    prove_signup_cas(database)
    prove_query_staging_and_replay(database)
    prove_event_and_effect_staging(database)
    prove_notification_rate_limit(database)
    assert not database.execute("PRAGMA foreign_key_check").fetchall()
    print("legacy analytics SQLite conformance: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
