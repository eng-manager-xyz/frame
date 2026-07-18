#!/usr/bin/env python3
"""Provider-free SQLite proof for Cap's desktop session handoff."""

from __future__ import annotations

import hashlib
import sqlite3
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
MINT = (
    ROOT
    / "apps/control-plane/queries/legacy_desktop_session/mint_desktop_key.sql"
).read_text(encoding="utf-8")
NOW = 1_700_000_000_000


def identifier(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def database() -> sqlite3.Connection:
    db = sqlite3.connect(":memory:")
    db.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        db.executescript(migration.read_text(encoding="utf-8"))
    return db


def add_user(db: sqlite3.Connection, number: int, *, active: bool = True) -> str:
    user_id = identifier(number)
    db.execute(
        """INSERT INTO users(
             id,email,display_name,status,created_at_ms,updated_at_ms,
             deleted_at_ms
           ) VALUES(?,?,?,?,?,?,?)""",
        (
            user_id,
            f"user-{number}@example.test",
            f"User {number}",
            "active" if active else "suspended",
            1,
            1,
            None,
        ),
    )
    return user_id


def main() -> None:
    db = database()
    actor = add_user(db, 1)
    inactive = add_user(db, 2, active=False)

    # The source-visible UUID is returned only by the carrier; D1 receives its
    # SHA-256 digest, client source, and audit row id.
    raw = str(uuid.UUID("12345678-1234-4abc-8def-123456789012"))
    digest = hashlib.sha256(raw.encode()).hexdigest()
    row_id = identifier(100)
    returned = db.execute(MINT, (row_id, actor, digest, NOW)).fetchone()
    assert returned is not None and returned["id"] == row_id
    stored = db.execute(
        """SELECT user_id,key_digest,name,scopes_json,legacy_source,
                  expires_at_ms,revoked_at_ms
           FROM auth_api_keys WHERE id=?""",
        (row_id,),
    ).fetchone()
    assert tuple(stored) == (
        actor,
        digest,
        "Cap desktop app",
        '["frame:read","frame:write"]',
        "desktop",
        None,
        None,
    )
    assert raw not in "|".join(str(value) for value in stored)

    # Inactive/deleted actors cannot mint, and digest uniqueness prevents one
    # raw credential from being rebound to another actor.
    assert db.execute(
        MINT, (identifier(101), inactive, hashlib.sha256(b"inactive").hexdigest(), NOW)
    ).fetchone() is None
    db.execute("UPDATE users SET deleted_at_ms=? WHERE id=?", (NOW, actor))
    assert db.execute(
        MINT, (identifier(102), actor, hashlib.sha256(b"deleted").hexdigest(), NOW)
    ).fetchone() is None
    db.execute("UPDATE users SET deleted_at_ms=NULL WHERE id=?", (actor,))
    try:
        db.execute(MINT, (identifier(103), actor, digest, NOW))
        raise AssertionError("duplicate raw desktop key digest was accepted")
    except sqlite3.IntegrityError:
        pass

    # The HTTP export query is bound to the authenticated session id and actor
    # and returns the earlier of idle/absolute expiry. It cannot select a
    # sibling session for the same user.
    operation_id = identifier(200)
    session_id = identifier(201)
    sibling_id = identifier(202)
    db.execute(
        """INSERT INTO auth_identities_v2(
             user_id,identity_revision,session_version,created_at_ms,
             updated_at_ms,revision,last_operation_id
           ) VALUES(?,1,0,1,1,0,?)""",
        (actor, operation_id),
    )
    session_values = (
        actor,
        1,
        "a" * 64,
        1,
        "b" * 64,
        "https://frame.engmanager.xyz",
        NOW - 1_000,
        NOW - 1_000,
        NOW + 60_000,
        NOW + 120_000,
        operation_id,
    )
    db.execute(
        """INSERT INTO auth_sessions_v2(
             id,family_id,user_id,client_kind,token_key_version,token_digest,
             csrf_key_version,csrf_digest,browser_origin,issued_at_ms,
             rotated_at_ms,idle_expires_at_ms,absolute_expires_at_ms,
             session_version,generation,state,revision,last_operation_id
           ) VALUES(?,? ,?,'browser',?,?,?,?,?,?,?,?,?,0,0,'active',0,?)""",
        (session_id, identifier(210), *session_values),
    )
    sibling_values = list(session_values)
    sibling_values[2] = "c" * 64
    sibling_values[4] = "d" * 64
    sibling_values[8] = NOW + 600_000
    sibling_values[9] = NOW + 600_000
    db.execute(
        """INSERT INTO auth_sessions_v2(
             id,family_id,user_id,client_kind,token_key_version,token_digest,
             csrf_key_version,csrf_digest,browser_origin,issued_at_ms,
             rotated_at_ms,idle_expires_at_ms,absolute_expires_at_ms,
             session_version,generation,state,revision,last_operation_id
           ) VALUES(?,? ,?,'browser',?,?,?,?,?,?,?,?,?,0,0,'active',0,?)""",
        (sibling_id, identifier(211), *sibling_values),
    )
    expiry_query = """SELECT CASE
      WHEN idle_expires_at_ms < absolute_expires_at_ms THEN idle_expires_at_ms
      ELSE absolute_expires_at_ms END AS expires_at_ms
      FROM auth_sessions_v2
      WHERE id=? AND user_id=? AND state='active'
        AND idle_expires_at_ms>? AND absolute_expires_at_ms>?
      LIMIT 1"""
    expiry = db.execute(
        expiry_query, (session_id, actor, NOW, NOW)
    ).fetchone()
    assert expiry is not None and expiry["expires_at_ms"] == NOW + 60_000
    assert db.execute(
        expiry_query, (session_id, inactive, NOW, NOW)
    ).fetchone() is None

    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
    print(
        "legacy desktop session SQLite conformance passed: active-actor "
        "digest-only desktop UUID mint, exact RETURNING row, and "
        "session-id-bound minimum expiry"
    )


if __name__ == "__main__":
    main()
