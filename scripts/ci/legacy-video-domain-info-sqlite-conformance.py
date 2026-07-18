#!/usr/bin/env python3
"""Provider-free D1/SQLite proof for the anonymous video domain-info route."""

from __future__ import annotations

import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_video_domain_info"

OWNER = "00000000-0000-4000-8000-000000000001"
SHARER = "00000000-0000-4000-8000-000000000002"
OWNER_ORG = "10000000-0000-4000-8000-000000000001"
SHARED_ORG = "10000000-0000-4000-8000-000000000002"
VIDEO = "20000000-0000-4000-8000-000000000001"


def sql(name: str) -> str:
    return (QUERIES / name).read_text(encoding="utf-8")


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:")
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def seed(connection: sqlite3.Connection) -> None:
    for user_id, email in [(OWNER, "owner@example.test"), (SHARER, "sharer@example.test")]:
        connection.execute(
            "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
            (user_id, email, email.split("@")[0], 1, 1),
        )
    for organization_id, owner_id, name in [
        (OWNER_ORG, OWNER, "Owner organization"),
        (SHARED_ORG, SHARER, "Shared organization"),
    ]:
        connection.execute(
            """INSERT INTO organizations(
                 id,owner_id,name,status,created_at_ms,updated_at_ms
               ) VALUES(?,?,?,'active',1,1)""",
            (organization_id, owner_id, name),
        )
        connection.execute(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,created_at_ms,updated_at_ms
               ) VALUES(?,?,'owner','active',1,1)""",
            (organization_id, owner_id),
        )
    connection.execute(
        """INSERT INTO legacy_org_custom_domain_projection_v1(
             organization_id,custom_domain,domain_verified_iso,source_row_digest,imported_at_ms
           ) VALUES(?,?,?,?,1)""",
        (OWNER_ORG, "owner.example", None, "a" * 64),
    )
    connection.execute(
        """INSERT INTO legacy_org_custom_domain_projection_v1(
             organization_id,custom_domain,domain_verified_iso,source_row_digest,imported_at_ms
           ) VALUES(?,?,?,?,1)""",
        (SHARED_ORG, "shared.example", "2026-07-17T19:00:00.000Z", "b" * 64),
    )
    connection.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id
           ) VALUES(?,?,'Domain video','ready',1,1,?)""",
        (VIDEO, OWNER, SHARED_ORG),
    )
    connection.execute(
        """INSERT INTO shared_videos(
             id,video_id,organization_id,folder_id,shared_by_user_id,sharing_mode,shared_at_ms
           ) VALUES(?,?,?,NULL,?,'organization',1)""",
        ("30000000-0000-4000-8000-000000000001", VIDEO, SHARED_ORG, SHARER),
    )
    connection.commit()


def main() -> None:
    connection = database()
    seed(connection)

    authority = connection.execute(sql("video_authority.sql"), (VIDEO,)).fetchall()
    assert len(authority) == 1
    assert authority[0]["owner_id"] == OWNER
    assert authority[0]["shared_organization_id"] == SHARED_ORG
    assert connection.execute(
        sql("video_authority.sql"), ("missing-video",)
    ).fetchall() == []

    shared = connection.execute(
        sql("organization_domain.sql"), (SHARED_ORG,)
    ).fetchone()
    assert shared["custom_domain"] == "shared.example"
    assert shared["domain_verified_iso"] == "2026-07-17T19:00:00.000Z"

    owner = connection.execute(sql("owner_domain.sql"), (OWNER,)).fetchone()
    assert owner["custom_domain"] == "owner.example"
    assert owner["domain_verified_iso"] is None

    connection.execute(
        "UPDATE shared_videos SET revoked_at_ms=2 WHERE video_id=?", (VIDEO,)
    )
    revoked = connection.execute(sql("video_authority.sql"), (VIDEO,)).fetchone()
    assert revoked["shared_organization_id"] is None

    print(
        "legacy video domain-info SQLite conformance passed: anonymous lookup, "
        "shared-before-owner precedence, ISO timestamp projection, missing video, and revoked share"
    )


if __name__ == "__main__":
    main()
