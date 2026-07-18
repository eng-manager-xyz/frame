#!/usr/bin/env python3
"""Provider-free D1 proof for Cap's scoped notification-list read."""

from __future__ import annotations

import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
QUERY_ROOT = ROOT / "apps/control-plane/queries/legacy_notification_read"
READ_ROWS = (QUERY_ROOT / "read_rows.sql").read_text(encoding="utf-8")
READ_COUNTS = (QUERY_ROOT / "read_counts.sql").read_text(encoding="utf-8")


def database() -> sqlite3.Connection:
    db = sqlite3.connect(":memory:")
    db.row_factory = sqlite3.Row
    db.executescript(
        """
        PRAGMA foreign_keys = ON;
        CREATE TABLE users(
          id TEXT PRIMARY KEY NOT NULL,
          display_name TEXT,
          status TEXT NOT NULL CHECK(status IN ('active','disabled')),
          deleted_at_ms INTEGER,
          active_organization_id TEXT,
          legacy_image_key TEXT
        );
        CREATE TABLE notifications(
          id TEXT PRIMARY KEY NOT NULL,
          organization_id TEXT,
          recipient_user_id TEXT NOT NULL REFERENCES users(id),
          type TEXT NOT NULL,
          data_json TEXT NOT NULL CHECK(json_valid(data_json)),
          created_at_ms INTEGER NOT NULL,
          read_at_ms INTEGER
        );
        """
    )
    return db


def insert_notification(
    db: sqlite3.Connection,
    notification_id: str,
    organization_id: str,
    recipient_id: str,
    notification_type: str,
    data_json: str,
    created_at_ms: int,
    read_at_ms: int | None = None,
) -> None:
    db.execute(
        """INSERT INTO notifications(
             id,organization_id,recipient_user_id,type,data_json,created_at_ms,read_at_ms
           ) VALUES (?,?,?,?,?,?,?)""",
        (
            notification_id,
            organization_id,
            recipient_id,
            notification_type,
            data_json,
            created_at_ms,
            read_at_ms,
        ),
    )


def main() -> int:
    assert READ_ROWS.count("?1") == 1
    assert READ_COUNTS.count("?1") == 1
    assert "n.read_at_ms IS NULL DESC" in READ_ROWS
    assert "n.created_at_ms DESC" in READ_ROWS
    assert "n.organization_id = actor.active_organization_id" in READ_ROWS
    assert "n.recipient_user_id = actor.id" in READ_ROWS

    db = database()
    db.executemany(
        """INSERT INTO users(
             id,display_name,status,deleted_at_ms,active_organization_id,legacy_image_key
           ) VALUES (?,?,?,?,?,?)""",
        [
            ("actor", "Actor", "active", None, "org-a", None),
            ("other", "Other", "active", None, "org-a", None),
            ("author", None, "active", None, "org-a", "avatars/author.png"),
            ("disabled", "Disabled", "disabled", None, "org-a", None),
            ("deleted", "Deleted", "active", 50, "org-a", None),
        ],
    )
    insert_notification(
        db,
        "comment-unread",
        "org-a",
        "actor",
        "comment",
        '{"videoId":"video-a","authorId":"author","comment":{"id":"c1","content":"hello"}}',
        100,
    )
    insert_notification(
        db,
        "anon-newer",
        "org-a",
        "actor",
        "anon_view",
        '{"videoId":"video-a","anonName":null,"location":null}',
        200,
    )
    insert_notification(
        db,
        "view-read",
        "org-a",
        "actor",
        "view",
        '{"videoId":"video-a","authorId":"author"}',
        300,
        301,
    )
    insert_notification(
        db,
        "wrong-org",
        "org-b",
        "actor",
        "reaction",
        '{"videoId":"video-b","authorId":"author","comment":{"id":"c2","content":"x"}}',
        400,
    )
    insert_notification(
        db,
        "wrong-actor",
        "org-a",
        "other",
        "reply",
        '{"videoId":"video-a","authorId":"author","comment":{"id":"c3","content":"x"}}',
        500,
    )
    insert_notification(
        db,
        "missing-author",
        "org-a",
        "actor",
        "view",
        '{"videoId":"video-a","authorId":"missing"}',
        50,
    )

    rows = db.execute(READ_ROWS, ("actor",)).fetchall()
    assert [row["id"] for row in rows] == [
        "anon-newer",
        "comment-unread",
        "missing-author",
        "view-read",
    ]
    authored = next(row for row in rows if row["id"] == "comment-unread")
    assert authored["author_id"] == "author"
    assert authored["author_name"] is None
    assert authored["author_image_key"] == "avatars/author.png"
    missing = next(row for row in rows if row["id"] == "missing-author")
    assert missing["author_id"] is None

    counts = {
        row["notification_type"]: row["notification_count"]
        for row in db.execute(READ_COUNTS, ("actor",)).fetchall()
    }
    assert counts == {"anon_view": 1, "comment": 1, "view": 2}
    assert db.execute(READ_ROWS, ("missing",)).fetchall() == []
    assert db.execute(READ_ROWS, ("disabled",)).fetchall() == []
    assert db.execute(READ_ROWS, ("deleted",)).fetchall() == []

    db.execute(
        "UPDATE users SET active_organization_id = 'org-b' WHERE id = 'actor'"
    )
    assert [row["id"] for row in db.execute(READ_ROWS, ("actor",)).fetchall()] == [
        "wrong-org"
    ]

    print(
        "Legacy notification-read SQLite conformance passed: actor/active-org scope, "
        "unread ordering, author joins, grouped counts, and cross-tenant exclusion."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
