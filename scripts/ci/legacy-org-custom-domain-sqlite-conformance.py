#!/usr/bin/env python3
"""Provider-free proof for Cap's lossless custom-domain projection and read."""

from __future__ import annotations

import sqlite3
import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
QUERY = (
    ROOT
    / "apps/control-plane/queries/legacy_org_custom_domain/read_for_actor.sql"
).read_text(encoding="utf-8")
UPSERT = (
    ROOT
    / "apps/control-plane/queries/legacy_org_custom_domain/upsert_projection.sql"
).read_text(encoding="utf-8")
API_KEY_ACTOR = (
    ROOT
    / "apps/control-plane/queries/legacy_org_custom_domain/api_key_actor.sql"
).read_text(encoding="utf-8")
MIGRATION = (
    ROOT
    / "apps/control-plane/migrations/0030_legacy_org_custom_domain_projection.sql"
).read_text(encoding="utf-8")


def database() -> sqlite3.Connection:
    db = sqlite3.connect(":memory:")
    db.executescript(
        """
        PRAGMA foreign_keys = ON;
        CREATE TABLE organizations(id TEXT PRIMARY KEY);
        CREATE TABLE users(
          id TEXT PRIMARY KEY,
          active_organization_id TEXT,
          status TEXT NOT NULL,
          deleted_at_ms INTEGER
        );
        CREATE TABLE auth_api_keys(
          id TEXT PRIMARY KEY,
          user_id TEXT NOT NULL REFERENCES users(id),
          key_digest TEXT NOT NULL UNIQUE,
          expires_at_ms INTEGER,
          revoked_at_ms INTEGER
        );
        """
    )
    db.executescript(MIGRATION)
    return db


def rows(
    db: sqlite3.Connection, actor_id: str
) -> list[tuple[int, int, str | None, str | None]]:
    return db.execute(QUERY, (actor_id,)).fetchall()


def upsert(
    db: sqlite3.Connection,
    organization_id: str,
    custom_domain: str | None,
    domain_verified_iso: str | None,
    digest_char: str,
    imported_at_ms: int,
) -> None:
    db.execute(
        UPSERT,
        (
            organization_id,
            custom_domain,
            domain_verified_iso,
            digest_char * 64,
            imported_at_ms,
        ),
    )


def main() -> int:
    db = database()
    verified = "2026-07-16T12:34:56.789Z"
    assert QUERY.count("?") == 1
    assert UPSERT.count("?") == 5
    assert API_KEY_ACTOR.count("?") == 2
    assert "organization_members" not in QUERY
    assert "storage_custom_domains_v1" not in QUERY
    assert "u.status" not in QUERY

    db.executemany(
        "INSERT INTO organizations VALUES (?)",
        [("org_a",), ("org_b",), ("org_c",), ("org_d",)],
    )
    db.executemany(
        "INSERT INTO users VALUES (?, ?, 'active', NULL)",
        [
            ("no_org", None),
            ("domain_only", "org_a"),
            ("verified", "org_b"),
            ("missing_projection", "org_c"),
            ("verified_only", "org_d"),
        ],
    )
    upsert(db, "org_a", "domain.example.com", None, "a", 1)
    upsert(db, "org_b", "https://verified.example.com", verified, "b", 2)
    upsert(db, "org_d", None, verified, "d", 3)

    active_key = "12345678-1234-1234-1234-123456789012"
    expired_key = "22345678-1234-1234-1234-123456789012"
    revoked_key = "32345678-1234-1234-1234-123456789012"
    inactive_key = "42345678-1234-1234-1234-123456789012"
    db.execute(
        "INSERT INTO users VALUES (?, ?, 'disabled', NULL)",
        ("inactive", "org_a"),
    )
    db.executemany(
        "INSERT INTO auth_api_keys VALUES (?, ?, ?, ?, ?)",
        [
            (
                "key_active",
                "verified",
                hashlib.sha256(active_key.encode()).hexdigest(),
                None,
                None,
            ),
            (
                "key_expired",
                "verified",
                hashlib.sha256(expired_key.encode()).hexdigest(),
                999,
                None,
            ),
            (
                "key_revoked",
                "verified",
                hashlib.sha256(revoked_key.encode()).hexdigest(),
                None,
                999,
            ),
            (
                "key_inactive",
                "inactive",
                hashlib.sha256(inactive_key.encode()).hexdigest(),
                None,
                None,
            ),
        ],
    )

    assert rows(db, "missing") == []
    assert rows(db, "no_org") == [(0, 0, None, None)]
    assert rows(db, "domain_only") == [(1, 1, "domain.example.com", None)]
    assert rows(db, "verified") == [
        (1, 1, "https://verified.example.com", verified)
    ]
    assert rows(db, "missing_projection") == [(1, 0, None, None)]
    assert rows(db, "verified_only") == [(1, 1, None, verified)]
    assert db.execute(
        API_KEY_ACTOR,
        (hashlib.sha256(active_key.encode()).hexdigest(), 1_000),
    ).fetchall() == [("key_active", "verified")]
    for rejected_key in (expired_key, revoked_key, inactive_key):
        assert db.execute(
            API_KEY_ACTOR,
            (hashlib.sha256(rejected_key.encode()).hexdigest(), 1_000),
        ).fetchall() == []
    assert db.execute(API_KEY_ACTOR, ("f" * 64, 1_000)).fetchall() == []

    # The import boundary preserves independent nullability and overwrites the
    # complete source projection atomically on repeated organization rows.
    upsert(db, "org_a", "", verified, "c", 4)
    assert rows(db, "domain_only") == [(1, 1, "", verified)]
    assert db.execute(
        """SELECT source_row_digest, imported_at_ms
           FROM legacy_org_custom_domain_projection_v1
           WHERE organization_id = 'org_a'"""
    ).fetchone() == ("c" * 64, 4)
    assert db.execute(
        """SELECT count(*) FROM legacy_org_custom_domain_projection_v1
           WHERE organization_id = 'org_a'"""
    ).fetchone() == (1,)

    for invalid_args in [
        ("org_c", "bad.example.com", None, "X" * 64, 5),
        ("org_c", "bad.example.com", "not-an-iso-timestamp", "e" * 64, 5),
    ]:
        try:
            db.execute(UPSERT, invalid_args)
        except sqlite3.IntegrityError:
            pass
        else:
            raise AssertionError(f"projection accepted invalid row: {invalid_args!r}")

    print(
        "Legacy org custom-domain SQLite conformance passed: lossless projection upsert, "
        "session/API-key actor derivation, active-organization read, independent nulls, "
        "and integrity bounds."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
