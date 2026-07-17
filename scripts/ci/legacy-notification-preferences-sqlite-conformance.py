#!/usr/bin/env python3
"""Provider-free proof for the raw notification-preferences actor read."""

from __future__ import annotations

import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
QUERY_DIR = ROOT / "apps/control-plane/queries/legacy_notification_preferences"
READ = (QUERY_DIR / "read_for_actor.sql").read_text(encoding="utf-8")


def database() -> sqlite3.Connection:
    db = sqlite3.connect(":memory:")
    db.executescript(
        """
        PRAGMA foreign_keys = ON;
        CREATE TABLE users(
          id TEXT PRIMARY KEY NOT NULL,
          preferences_json TEXT CHECK (
            preferences_json IS NULL OR json_valid(preferences_json)
          )
        );
        """
    )
    return db


def rows(db: sqlite3.Connection, actor_id: str) -> list[tuple[str | None]]:
    return db.execute(READ, (actor_id,)).fetchall()


def main() -> int:
    db = database()
    assert READ.count("?") == 1
    for forbidden in (
        "organization_members",
        "active_organization_id",
        "u.status",
        "u.deleted_at_ms",
    ):
        assert forbidden not in READ

    db.executemany(
        "INSERT INTO users(id, preferences_json) VALUES (?, NULL)",
        [("null_source",), ("valid_source",), ("partial_source",)],
    )
    valid = (
        '{"notifications":{"pauseComments":true,"pauseReplies":false,'
        '"pauseViews":true,"pauseReactions":false}}'
    )
    partial = '{"notifications":{"pauseComments":true}}'
    db.execute(
        "UPDATE users SET preferences_json = ? WHERE id = 'valid_source'", (valid,)
    )
    db.execute(
        "UPDATE users SET preferences_json = ? WHERE id = 'partial_source'", (partial,)
    )

    assert rows(db, "missing") == []
    assert rows(db, "null_source") == [(None,)]
    assert rows(db, "valid_source") == [(valid,)]
    assert rows(db, "partial_source") == [(partial,)]

    replacement = "null"
    db.execute(
        "UPDATE users SET preferences_json = ? WHERE id = 'valid_source'",
        (replacement,),
    )
    assert rows(db, "valid_source") == [(replacement,)]

    try:
        db.execute(
            "UPDATE users SET preferences_json = 'not-json' WHERE id = 'valid_source'"
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("users authority accepted invalid JSON")
    assert rows(db, "valid_source") == [(replacement,)]

    print(
        "Legacy notification-preferences SQLite conformance passed: actor-only raw JSON "
        "read, missing/null preservation, and authority JSON integrity."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
