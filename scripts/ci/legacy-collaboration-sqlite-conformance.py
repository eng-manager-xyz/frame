#!/usr/bin/env python3
"""SQLite conformance for Cap's six retained collaboration mutations."""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
import tempfile
import uuid
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_collaboration"
MIGRATION = MIGRATIONS / "0043_legacy_collaboration_expand.sql"

ACTOR = "00000000-0000-7000-8000-000000000001"
OTHER = "00000000-0000-7000-8000-000000000002"
OUTSIDER = "00000000-0000-7000-8000-000000000003"
ORGANIZATION = "00000000-0000-7000-8000-000000000101"
FOREIGN_ORGANIZATION = "00000000-0000-7000-8000-000000000102"
OWNED_VIDEO = "00000000-0000-7000-8000-000000000201"
SHARED_VIDEO = "00000000-0000-7000-8000-000000000202"
FOREIGN_VIDEO = "00000000-0000-7000-8000-000000000203"
SHARE = "00000000-0000-7000-8000-000000000301"

LEGACY_ACTOR = "0123456789abcde"
LEGACY_OTHER = "1123456789abcde"
LEGACY_OWNED_VIDEO = "2123456789abcde"
LEGACY_SHARED_VIDEO = "3123456789abcde"
LEGACY_FOREIGN_VIDEO = "4123456789abcde"
NOW = 1_700_000_000_000

QUERY_NAMES = (
    "operation_by_key", "operation_claim", "operation_complete",
    "tenant_authority_snapshot", "tenant_authority_assert",
    "user_alias_insert", "user_alias_assert", "video_authority_snapshot",
    "video_authority_assert", "comment_insert", "changes_assert",
    "create_receipt_insert", "delete_targets_insert", "authored_target_assert",
    "delete_bound_assert", "notification_targets_insert", "notification_bound_assert",
    "comments_delete",
    "comments_deleted_assert", "notifications_delete",
    "notifications_deleted_assert", "delete_receipt_insert", "effect_insert",
    "audit_insert", "durable_receipt_assert", "assertion_cleanup",
    "notification_attempt_insert",
)
SQL = {name: (QUERIES / f"{name}.sql").read_text(encoding="utf-8") for name in QUERY_NAMES}


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def uid(serial: int) -> str:
    return f"00000000-0000-7000-8000-{serial:012d}"


def connect(path: Path | None = None) -> sqlite3.Connection:
    database = sqlite3.connect(path or ":memory:", isolation_level=None)
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    database.execute("PRAGMA busy_timeout = 15000")
    return database


def migrated_database(path: Path | None = None) -> sqlite3.Connection:
    database = connect(path)
    migrations = [
        item for item in sorted(MIGRATIONS.glob("*.sql"))
        if int(item.name[:4]) <= 43
    ]
    for migration in migrations:
        database.executescript(migration.read_text(encoding="utf-8"))
        violations = database.execute("PRAGMA foreign_key_check").fetchall()
        assert not violations, f"{migration.name} introduced {violations}"
    return database


def seed_fixture(database: sqlite3.Connection) -> None:
    database.execute("BEGIN")
    try:
        database.executemany(
            """INSERT INTO users(
                 id,email,display_name,created_at_ms,updated_at_ms,status,
                 active_organization_id,organization_preference_revision
               ) VALUES (?1,?2,?3,1,1,'active',?4,7)""",
            (
                (ACTOR, "actor@example.invalid", "Actor", ORGANIZATION),
                (OTHER, "other@example.invalid", "Other", ORGANIZATION),
                (OUTSIDER, "outside@example.invalid", "Outside", FOREIGN_ORGANIZATION),
            ),
        )
        database.executemany(
            """INSERT INTO organizations(
                 id,owner_id,name,status,created_at_ms,updated_at_ms,
                 revision,authority_version
               ) VALUES (?1,?2,?3,'active',1,1,11,13)""",
            (
                (ORGANIZATION, ACTOR, "primary"),
                (FOREIGN_ORGANIZATION, OUTSIDER, "foreign"),
            ),
        )
        database.executemany(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,created_at_ms,updated_at_ms,
                 revision,authority_version
               ) VALUES (?1,?2,?3,'active',1,1,5,9)""",
            (
                (ORGANIZATION, ACTOR, "owner"),
                (ORGANIZATION, OTHER, "member"),
                (FOREIGN_ORGANIZATION, OUTSIDER, "owner"),
            ),
        )
        database.executemany(
            """INSERT INTO videos(
                 id,owner_id,title,state,created_at_ms,updated_at_ms,
                 organization_id,revision
               ) VALUES (?1,?2,?3,'ready',1,1,?4,17)""",
            (
                (OWNED_VIDEO, ACTOR, "owned", ORGANIZATION),
                (SHARED_VIDEO, OTHER, "shared", ORGANIZATION),
                (FOREIGN_VIDEO, OUTSIDER, "foreign", FOREIGN_ORGANIZATION),
            ),
        )
        database.execute(
            """INSERT INTO shared_videos(
                 id,video_id,organization_id,folder_id,shared_by_user_id,
                 sharing_mode,shared_at_ms,revision
               ) VALUES (?1,?2,?3,NULL,?4,'organization',1,19)""",
            (SHARE, SHARED_VIDEO, ORGANIZATION, OTHER),
        )
        database.executemany(
            """INSERT INTO legacy_collaboration_user_aliases_v1(
                 legacy_user_id,mapped_user_id,image_url,provenance,
                 created_at_ms,refreshed_at_ms
               ) VALUES (?1,?2,?3,'cap_backfill',1,1)""",
            ((LEGACY_ACTOR, ACTOR, "https://images.invalid/actor"), (LEGACY_OTHER, OTHER, None)),
        )
        database.executemany(
            """INSERT INTO legacy_collaboration_video_aliases_v1(
                 legacy_video_id,mapped_video_id,provenance,created_at_ms
               ) VALUES (?1,?2,'cap_backfill',1)""",
            (
                (LEGACY_OWNED_VIDEO, OWNED_VIDEO),
                (LEGACY_SHARED_VIDEO, SHARED_VIDEO),
                (LEGACY_FOREIGN_VIDEO, FOREIGN_VIDEO),
            ),
        )
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


def seeded_database() -> sqlite3.Connection:
    database = migrated_database()
    seed_fixture(database)
    return database


def authority(database: sqlite3.Connection, actor: str = ACTOR, organization: str = ORGANIZATION) -> sqlite3.Row:
    rows = database.execute(SQL["tenant_authority_snapshot"], (actor, organization)).fetchall()
    assert len(rows) == 1
    return rows[0]


def authority_assert_args(operation_id: str, row: sqlite3.Row, actor: str = ACTOR, organization: str = ORGANIZATION) -> tuple[object, ...]:
    return (
        operation_id, actor, organization, row["selection_revision"],
        row["user_updated_at_ms"], row["organization_revision"],
        row["organization_authority_version"], row["membership_role"],
        row["membership_state"], row["membership_revision"],
        row["membership_authority_version"], row["author_name"],
        row["legacy_author_id"], row["author_image"],
    )


def claim(database: sqlite3.Connection, operation_id: str, action: str, key: str, request: str) -> None:
    database.execute(SQL["operation_claim"], (operation_id, ORGANIZATION, ACTOR, action, digest(key), digest(request), NOW))


def finish(database: sqlite3.Connection, operation_id: str, action: str, request: str, timing: str, rollback: int, path: str) -> None:
    database.execute(SQL["effect_insert"], (operation_id, timing, rollback, path, NOW))
    database.execute(SQL["changes_assert"], (operation_id, "effect_inserted", 1))
    database.execute(SQL["audit_insert"], (str(uuid.uuid4()), operation_id, ORGANIZATION, ACTOR, action, digest(request), NOW))
    database.execute(SQL["changes_assert"], (operation_id, "audit_inserted", 1))
    database.execute(SQL["operation_complete"], (operation_id, NOW))
    database.execute(SQL["changes_assert"], (operation_id, "operation_complete", 1))
    database.execute(SQL["durable_receipt_assert"], (operation_id,))
    database.execute(SQL["assertion_cleanup"], (operation_id,))


def create_comment(
    database: sqlite3.Connection,
    *,
    serial: int,
    action: str = "legacy.collaboration.mobile_create_comment",
    legacy_video_id: str = LEGACY_OWNED_VIDEO,
    content: str = "hello",
    parent: str | None = None,
    kind: str = "text",
    notification_kind: str = "comment",
    mobile_authority: bool = True,
    response_image: str | None = "https://images.invalid/actor",
    attempt_notification: bool = True,
) -> tuple[str, str]:
    operation_id, comment_id = uid(10_000 + serial), f"{serial:015x}"[-15:]
    mapped_comment_id = uid(20_000 + serial)
    key, request = f"create-key-{serial}", f"create-request-{serial}"
    auth = authority(database)
    video = None
    if mobile_authority:
        rows = database.execute(SQL["video_authority_snapshot"], (legacy_video_id, ACTOR, ORGANIZATION)).fetchall()
        assert len(rows) == 1
        video = rows[0]
    path = f"/s/{legacy_video_id}" if action == "legacy.collaboration.web_new_comment_action" else ""
    database.execute("BEGIN IMMEDIATE")
    try:
        claim(database, operation_id, action, key, request)
        database.execute(SQL["tenant_authority_assert"], authority_assert_args(operation_id, auth))
        database.execute(SQL["user_alias_insert"], (auth["legacy_author_id"], ACTOR, auth["author_image"], auth["alias_provenance"], NOW))
        database.execute(SQL["user_alias_assert"], (operation_id, auth["legacy_author_id"], ACTOR))
        mapped_video = None
        if video is not None:
            mapped_video = video["mapped_video_id"]
            database.execute(
                SQL["video_authority_assert"],
                (operation_id, legacy_video_id, mapped_video, video["owner_id"], ACTOR, ORGANIZATION, video["video_revision"], video["shared_revision"], video["authority_kind"]),
            )
        database.execute(
            SQL["comment_insert"],
            (comment_id, mapped_comment_id, legacy_video_id, mapped_video, ACTOR, auth["legacy_author_id"], kind, content, 1.25, parent, notification_kind, NOW, action, operation_id),
        )
        database.execute(SQL["changes_assert"], (operation_id, "comment_inserted", 1))
        database.execute(SQL["create_receipt_insert"], (operation_id, auth["author_name"], response_image, path, NOW))
        database.execute(SQL["changes_assert"], (operation_id, "receipt_inserted", 1))
        finish(database, operation_id, action, request, "after_insert_best_effort", 0, path)
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")
    if attempt_notification:
        try:
            database.execute(SQL["notification_attempt_insert"], (operation_id, comment_id, notification_kind, NOW))
        except sqlite3.DatabaseError:
            pass
    return operation_id, comment_id


def seed_comment(database: sqlite3.Connection, legacy_id: str, *, author: str = ACTOR, legacy_author: str = LEGACY_ACTOR, parent: str | None = None, video: str = LEGACY_OWNED_VIDEO) -> None:
    database.execute(
        """INSERT INTO legacy_collaboration_comments_v1(
             legacy_comment_id,mapped_comment_id,legacy_video_id,mapped_video_id,
             author_user_id,legacy_author_id,comment_kind,content,
             legacy_parent_comment_id,notification_kind,created_at_ms,updated_at_ms,
             source_action,last_operation_id
           ) VALUES (?1,?2,?3,?4,?5,?6,'text','seed',?7,'comment',1,1,
             'legacy.collaboration.cap_backfill',?8)""",
        (legacy_id, uid(30_000 + int(legacy_id[0], 16)), video, OWNED_VIDEO, author, legacy_author, parent, uid(40_000 + int(legacy_id[0], 16))),
    )


def seed_notification(database: sqlite3.Connection, serial: int, kind: str, comment_id: str, *, parent_id: str | None = None) -> str:
    notification_id = uid(50_000 + serial)
    comment = {"id": comment_id}
    if parent_id is not None:
        comment["parentCommentId"] = parent_id
    database.execute(
        """INSERT INTO notifications(
             id,organization_id,recipient_user_id,type,deduplication_key,
             data_json,created_at_ms
           ) VALUES (?1,?2,?3,?4,?5,?6,1)""",
        (notification_id, ORGANIZATION, OTHER, kind, f"notification-{serial}", json.dumps({"comment": comment})),
    )
    return notification_id


def delete_comment(database: sqlite3.Connection, *, serial: int, action: str, target: str, caller_parent: str | None = None, caller_video: str | None = None) -> str:
    operation_id = uid(60_000 + serial)
    key, request = f"delete-key-{serial}", f"delete-request-{serial}"
    auth = authority(database)
    route_mode = "route" if action == "legacy.collaboration.web_delete_comment_route" else "exact"
    selector = None
    if action == "legacy.collaboration.web_delete_comment_action":
        selector = "reply_by_comment_id" if caller_parent else "root_comment_and_replies_by_parent_id"
    path = f"/s/{caller_video}" if caller_video is not None else ""
    timing, rollback = ("same_delete_transaction", 1) if selector else ("none", 0)
    database.execute("BEGIN IMMEDIATE")
    try:
        claim(database, operation_id, action, key, request)
        database.execute(SQL["tenant_authority_assert"], authority_assert_args(operation_id, auth))
        database.execute(SQL["delete_targets_insert"], (operation_id, ACTOR, target, route_mode))
        database.execute(SQL["authored_target_assert"], (operation_id,))
        database.execute(SQL["delete_bound_assert"], (operation_id,))
        database.execute(SQL["notification_targets_insert"], (operation_id, selector, target))
        database.execute(SQL["notification_bound_assert"], (operation_id,))
        database.execute(SQL["comments_delete"], (operation_id,))
        database.execute(SQL["comments_deleted_assert"], (operation_id,))
        database.execute(SQL["notifications_delete"], (operation_id,))
        database.execute(SQL["notifications_deleted_assert"], (operation_id,))
        database.execute(SQL["delete_receipt_insert"], (operation_id, target, selector, path, NOW))
        database.execute(SQL["changes_assert"], (operation_id, "receipt_inserted", 1))
        finish(database, operation_id, action, request, timing, rollback, path)
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")
    return operation_id


def test_full_migration_chain_and_query_inventory() -> None:
    database = migrated_database()
    assert database.execute("PRAGMA foreign_key_check").fetchall() == []
    tables = {row[0] for row in database.execute("SELECT name FROM sqlite_master WHERE type='table'")}
    assert "legacy_collaboration_comments_v1" in tables
    assert "legacy_collaboration_notification_attempts_v1" in tables
    assert len(list(QUERIES.glob("*.sql"))) == 28


def test_mobile_owner_and_shared_authority() -> None:
    database = seeded_database()
    _, owned = create_comment(database, serial=1)
    _, shared = create_comment(database, serial=2, legacy_video_id=LEGACY_SHARED_VIDEO)
    assert database.execute("SELECT COUNT(*) FROM legacy_collaboration_comments_v1 WHERE legacy_comment_id IN (?1,?2)", (owned, shared)).fetchone()[0] == 2
    database.execute("UPDATE shared_videos SET revoked_at_ms=?1 WHERE id=?2", (NOW, SHARE))
    assert database.execute(SQL["video_authority_snapshot"], (LEGACY_SHARED_VIDEO, ACTOR, ORGANIZATION)).fetchall() == []
    assert database.execute(SQL["video_authority_snapshot"], (LEGACY_FOREIGN_VIDEO, ACTOR, ORGANIZATION)).fetchall() == []


def test_create_notification_failure_is_post_commit_and_swallowed() -> None:
    database = seeded_database()
    database.executescript(
        """CREATE TRIGGER inject_collaboration_handoff_failure
           BEFORE INSERT ON legacy_collaboration_notification_attempts_v1
           BEGIN SELECT RAISE(ABORT, 'injected provider failure'); END;"""
    )
    operation_id, comment_id = create_comment(database, serial=3)
    assert database.execute("SELECT state FROM legacy_collaboration_operations_v1 WHERE operation_id=?1", (operation_id,)).fetchone()[0] == "complete"
    assert database.execute("SELECT COUNT(*) FROM legacy_collaboration_comments_v1 WHERE legacy_comment_id=?1", (comment_id,)).fetchone()[0] == 1
    assert database.execute("SELECT COUNT(*) FROM legacy_collaboration_notification_attempts_v1 WHERE operation_id=?1", (operation_id,)).fetchone()[0] == 0


def test_new_comment_preserves_whitespace_empty_root_and_orphan_video_parent() -> None:
    database = seeded_database()
    operation_id, comment_id = create_comment(
        database, serial=4, action="legacy.collaboration.web_new_comment_action",
        legacy_video_id="orphan-video", content="   ", parent="",
        mobile_authority=False, response_image="caller-image",
    )
    row = database.execute("SELECT * FROM legacy_collaboration_comments_v1 WHERE legacy_comment_id=?1", (comment_id,)).fetchone()
    assert row["content"] == "   " and row["legacy_parent_comment_id"] == ""
    assert row["legacy_video_id"] == "orphan-video" and row["mapped_video_id"] is None
    receipt = database.execute("SELECT * FROM legacy_collaboration_receipts_v1 WHERE operation_id=?1", (operation_id,)).fetchone()
    assert receipt["author_image"] == "caller-image"
    assert receipt["notification_kind"] == "comment"
    assert receipt["revalidation_path"] == "/s/orphan-video"
    _, orphan = create_comment(
        database, serial=5, action="legacy.collaboration.web_new_comment_action",
        legacy_video_id="cross-video", parent="missing-parent", notification_kind="reply",
        mobile_authority=False,
    )
    assert database.execute("SELECT legacy_parent_comment_id FROM legacy_collaboration_comments_v1 WHERE legacy_comment_id=?1", (orphan,)).fetchone()[0] == "missing-parent"


def test_mobile_delete_is_exact_and_leaves_child() -> None:
    database = seeded_database()
    target, child = "5123456789abcde", "6123456789abcde"
    seed_comment(database, target)
    seed_comment(database, child, parent=target)
    operation_id = delete_comment(database, serial=6, action="legacy.collaboration.mobile_delete_comment", target=target)
    assert database.execute("SELECT COUNT(*) FROM legacy_collaboration_comments_v1 WHERE legacy_comment_id=?1", (target,)).fetchone()[0] == 0
    assert database.execute("SELECT COUNT(*) FROM legacy_collaboration_comments_v1 WHERE legacy_comment_id=?1", (child,)).fetchone()[0] == 1
    receipt = database.execute("SELECT * FROM legacy_collaboration_receipts_v1 WHERE operation_id=?1", (operation_id,)).fetchone()
    assert receipt["deleted_comment_count"] == 1 and receipt["deleted_notification_count"] == 0


def test_web_route_deletes_only_authored_target_and_authored_direct_replies() -> None:
    database = seeded_database()
    target, direct, other_reply, grandchild = "7123456789abcde", "8123456789abcde", "9123456789abcde", "a123456789abcde"
    seed_comment(database, target)
    seed_comment(database, direct, parent=target)
    seed_comment(database, other_reply, author=OTHER, legacy_author=LEGACY_OTHER, parent=target)
    seed_comment(database, grandchild, parent=direct)
    operation_id = delete_comment(database, serial=7, action="legacy.collaboration.web_delete_comment_route", target=target)
    remaining = {row[0] for row in database.execute("SELECT legacy_comment_id FROM legacy_collaboration_comments_v1")}
    assert target not in remaining and direct not in remaining
    assert other_reply in remaining and grandchild in remaining
    assert database.execute("SELECT deleted_comment_count FROM legacy_collaboration_receipts_v1 WHERE operation_id=?1", (operation_id,)).fetchone()[0] == 2


def test_action_reply_selector_uses_caller_parent_not_database_parent() -> None:
    database = seeded_database()
    target = "b123456789abcde"
    seed_comment(database, target, parent=None)
    exact = seed_notification(database, 1, "reply", target)
    root = seed_notification(database, 2, "comment", target)
    child = seed_notification(database, 3, "reply", "c123456789abcde", parent_id=target)
    operation_id = delete_comment(database, serial=8, action="legacy.collaboration.web_delete_comment_action", target=target, caller_parent="caller-parent", caller_video="untrusted-video")
    remaining = {row[0] for row in database.execute("SELECT id FROM notifications")}
    assert exact not in remaining and root in remaining and child in remaining
    receipt = database.execute("SELECT * FROM legacy_collaboration_receipts_v1 WHERE operation_id=?1", (operation_id,)).fetchone()
    assert receipt["notification_selector"] == "reply_by_comment_id"
    assert receipt["revalidation_path"] == "/s/untrusted-video"


def test_action_root_selector_uses_falsy_caller_even_for_stored_reply() -> None:
    database = seeded_database()
    target = "c123456789abcde"
    seed_comment(database, target, parent="stored-parent")
    exact_reply = seed_notification(database, 4, "reply", target)
    root = seed_notification(database, 5, "comment", target)
    child = seed_notification(database, 6, "reply", "d123456789abcde", parent_id=target)
    operation_id = delete_comment(database, serial=9, action="legacy.collaboration.web_delete_comment_action", target=target, caller_parent="", caller_video=LEGACY_FOREIGN_VIDEO)
    remaining = {row[0] for row in database.execute("SELECT id FROM notifications")}
    assert exact_reply in remaining and root not in remaining and child not in remaining
    receipt = database.execute("SELECT * FROM legacy_collaboration_receipts_v1 WHERE operation_id=?1", (operation_id,)).fetchone()
    assert receipt["notification_selector"] == "root_comment_and_replies_by_parent_id"
    assert receipt["revalidation_path"] == f"/s/{LEGACY_FOREIGN_VIDEO}"


def test_action_notification_delete_failure_rolls_back_comment_and_operation() -> None:
    database = seeded_database()
    target = "d123456789abcde"
    seed_comment(database, target)
    notification_id = seed_notification(database, 7, "comment", target)
    database.executescript(
        """CREATE TRIGGER inject_notification_delete_failure
           BEFORE DELETE ON notifications
           BEGIN SELECT RAISE(ABORT, 'injected notification failure'); END;"""
    )
    try:
        delete_comment(database, serial=10, action="legacy.collaboration.web_delete_comment_action", target=target, caller_video=LEGACY_OWNED_VIDEO)
    except sqlite3.DatabaseError as error:
        assert "injected notification failure" in str(error)
    else:
        raise AssertionError("action notification failure did not abort")
    assert database.execute("SELECT COUNT(*) FROM legacy_collaboration_comments_v1 WHERE legacy_comment_id=?1", (target,)).fetchone()[0] == 1
    assert database.execute("SELECT COUNT(*) FROM notifications WHERE id=?1", (notification_id,)).fetchone()[0] == 1
    assert database.execute("SELECT COUNT(*) FROM legacy_collaboration_operations_v1 WHERE operation_id=?1", (uid(60_010),)).fetchone()[0] == 0


def test_missing_or_wrong_author_target_fails_closed_without_claim() -> None:
    database = seeded_database()
    target = "e123456789abcde"
    seed_comment(database, target, author=OTHER, legacy_author=LEGACY_OTHER)
    for serial, missing in ((11, target), (12, "f123456789abcde")):
        try:
            delete_comment(database, serial=serial, action="legacy.collaboration.mobile_delete_comment", target=missing)
        except sqlite3.DatabaseError as error:
            assert "frame_legacy_collaboration_target_v1" in str(error)
        else:
            raise AssertionError("unauthorized target was deleted")
        assert database.execute("SELECT COUNT(*) FROM legacy_collaboration_operations_v1 WHERE operation_id=?1", (uid(60_000 + serial),)).fetchone()[0] == 0


def test_stale_authority_assertion_rolls_back_claim() -> None:
    database = seeded_database()
    auth = authority(database)
    operation_id = uid(70_001)
    database.execute("UPDATE users SET organization_preference_revision=8 WHERE id=?1", (ACTOR,))
    database.execute("BEGIN")
    try:
        claim(database, operation_id, "legacy.collaboration.mobile_delete_comment", "stale-key", "stale-request")
        database.execute(SQL["tenant_authority_assert"], authority_assert_args(operation_id, auth))
    except sqlite3.DatabaseError as error:
        database.execute("ROLLBACK")
        assert "frame_legacy_collaboration_authority_v1" in str(error)
    else:
        raise AssertionError("stale authority was accepted")
    assert database.execute("SELECT COUNT(*) FROM legacy_collaboration_operations_v1 WHERE operation_id=?1", (operation_id,)).fetchone()[0] == 0


def test_delete_and_notification_staging_bounds_fail_closed() -> None:
    database = seeded_database()
    cases = (
        (
            uid(75_001),
            """WITH RECURSIVE sequence(value) AS (
                 VALUES(1) UNION ALL SELECT value + 1 FROM sequence WHERE value < 100001
               )
               INSERT INTO legacy_collaboration_delete_targets_v1(
                 operation_id,legacy_comment_id,target_role,ordinal
               )
               SELECT ?1, printf('%015x', value),
                      CASE value WHEN 1 THEN 'target' ELSE 'authored_direct_reply' END,
                      value - 1
               FROM sequence""",
            SQL["delete_bound_assert"],
        ),
        (
            uid(75_002),
            """WITH RECURSIVE sequence(value) AS (
                 VALUES(1) UNION ALL SELECT value + 1 FROM sequence WHERE value < 100001
               )
               INSERT INTO legacy_collaboration_notification_targets_v1(
                 operation_id,notification_id,notification_type
               )
               SELECT ?1, printf('notification-%015x', value), 'reply'
               FROM sequence""",
            SQL["notification_bound_assert"],
        ),
    )
    for serial, (operation_id, stage_sql, assertion_sql) in enumerate(cases, 1):
        database.execute("BEGIN")
        try:
            claim(
                database,
                operation_id,
                "legacy.collaboration.web_delete_comment_action",
                f"bound-key-{serial}",
                f"bound-request-{serial}",
            )
            database.execute(stage_sql, (operation_id,))
            database.execute(assertion_sql, (operation_id,))
        except sqlite3.DatabaseError as error:
            database.execute("ROLLBACK")
            assert "frame_legacy_collaboration_corrupt_v1" in str(error)
        else:
            raise AssertionError("100001st staged row did not fail closed")
        assert database.execute(
            "SELECT COUNT(*) FROM legacy_collaboration_operations_v1 WHERE operation_id=?1",
            (operation_id,),
        ).fetchone()[0] == 0


def test_replay_identity_conflict_and_receipt_immutability() -> None:
    database = seeded_database()
    operation_id, _ = create_comment(database, serial=13)
    row = database.execute(SQL["operation_by_key"], (ORGANIZATION, ACTOR, "legacy.collaboration.mobile_create_comment", digest("create-key-13"))).fetchone()
    assert row["operation_id"] == operation_id and row["state"] == "complete"
    assert row["request_digest"] == digest("create-request-13")
    try:
        database.execute("UPDATE legacy_collaboration_receipts_v1 SET author_name='tampered' WHERE operation_id=?1", (operation_id,))
    except sqlite3.DatabaseError as error:
        assert "frame_legacy_collaboration_receipt_immutable_v1" in str(error)
    else:
        raise AssertionError("receipt mutation succeeded")
    try:
        claim(database, uid(80_001), "legacy.collaboration.mobile_create_comment", "create-key-13", "different-request")
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("duplicate scoped idempotency key was accepted")


TESTS: tuple[Callable[[], None], ...] = (
    test_full_migration_chain_and_query_inventory,
    test_mobile_owner_and_shared_authority,
    test_create_notification_failure_is_post_commit_and_swallowed,
    test_new_comment_preserves_whitespace_empty_root_and_orphan_video_parent,
    test_mobile_delete_is_exact_and_leaves_child,
    test_web_route_deletes_only_authored_target_and_authored_direct_replies,
    test_action_reply_selector_uses_caller_parent_not_database_parent,
    test_action_root_selector_uses_falsy_caller_even_for_stored_reply,
    test_action_notification_delete_failure_rolls_back_comment_and_operation,
    test_missing_or_wrong_author_target_fails_closed_without_claim,
    test_stale_authority_assertion_rolls_back_claim,
    test_delete_and_notification_staging_bounds_fail_closed,
    test_replay_identity_conflict_and_receipt_immutability,
)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", "--evidence-out", dest="evidence_out", type=Path)
    args = parser.parse_args()
    for test in TESTS:
        test()
        print(f"PASS {test.__name__}")
    evidence = {
        "schema": "frame.legacy-collaboration-sqlite-conformance.v1",
        "migration": MIGRATION.name,
        "migration_sha256": hashlib.sha256(MIGRATION.read_bytes()).hexdigest(),
        "query_count": len(list(QUERIES.glob("*.sql"))),
        "query_surface_sha256": hashlib.sha256("".join(
            f"{path.name}\0{hashlib.sha256(path.read_bytes()).hexdigest()}\n"
            for path in sorted(QUERIES.glob("*.sql"))
        ).encode()).hexdigest(),
        "tests": [test.__name__ for test in TESTS],
        "passed": len(TESTS),
    }
    if args.evidence_out:
        args.evidence_out.parent.mkdir(parents=True, exist_ok=True)
        args.evidence_out.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(evidence, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
