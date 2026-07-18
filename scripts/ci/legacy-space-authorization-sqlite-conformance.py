#!/usr/bin/env python3
"""Provider-free D1/SQLite proof for legacy space authorization reads."""

from __future__ import annotations

import hashlib
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_space_authorization"

ACTOR = "00000000-0000-4000-8000-000000000001"
OWNER = "00000000-0000-4000-8000-000000000002"
CREATOR = "00000000-0000-4000-8000-000000000003"
COLLISION_USER = "00000000-0000-4000-8000-000000000004"
FOREIGN_OWNER = "00000000-0000-4000-8000-000000000005"
ORG = "10000000-0000-4000-8000-000000000001"
FOREIGN_ORG = "10000000-0000-4000-8000-000000000002"
SPACE = "20000000-0000-4000-8000-000000000001"
FOREIGN_SPACE = "20000000-0000-4000-8000-000000000002"
LEGACY_ORG = "0123456789abcde"
LEGACY_FOREIGN_ORG = "1123456789abcde"
LEGACY_SPACE = "2123456789abcde"
LEGACY_FOREIGN_SPACE = "3123456789abcde"
LEGACY_OWNER = "4123456789abcde"
LEGACY_OWNER_MEMBER = "5123456789abcde"
ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"


def sql(name: str) -> str:
    return (QUERIES / name).read_text(encoding="utf-8")


def native_alias_candidate(user_id: str, attempt: int) -> str:
    digest = hashlib.sha256(
        b"frame-space-authorization-native-user-alias-v1\0"
        + user_id.encode()
        + bytes((0, attempt))
    ).digest()
    encoded: list[str] = []
    bit_offset = 0
    for _ in range(15):
        byte_index, shift = divmod(bit_offset, 8)
        pair = digest[byte_index] << 8
        if byte_index + 1 < len(digest):
            pair |= digest[byte_index + 1]
        encoded.append(ALPHABET[(pair >> (11 - shift)) & 31])
        bit_offset += 5
    return "".join(encoded)


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:")
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def seed(connection: sqlite3.Connection) -> None:
    for user_id, name in [
        (ACTOR, "actor"),
        (OWNER, "owner"),
        (CREATOR, "creator"),
        (COLLISION_USER, "collision"),
        (FOREIGN_OWNER, "foreign"),
    ]:
        connection.execute(
            "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
            (user_id, f"{name}@example.test", name, 1, 1),
        )
    for organization_id, owner_id, name in [
        (ORG, OWNER, "Organization"),
        (FOREIGN_ORG, FOREIGN_OWNER, "Foreign organization"),
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
    # The role-normalization unit proof separately covers a dirty non-owner
    # `owner` value; Frame's native integrity trigger correctly forbids
    # creating that state through D1.
    connection.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,created_at_ms,updated_at_ms
           ) VALUES(?,?,'admin','active',1,1)""",
        (ORG, ACTOR),
    )
    connection.execute("UPDATE users SET active_organization_id=? WHERE id=?", (ORG, ACTOR))
    for organization_id, legacy_id in [
        (ORG, LEGACY_ORG),
        (FOREIGN_ORG, LEGACY_FOREIGN_ORG),
    ]:
        connection.execute(
            """INSERT INTO legacy_user_account_organization_ids_v1(
                 organization_id,legacy_organization_id,recorded_at_ms,last_operation_id
               ) VALUES(?,?,1,?)""",
            (organization_id, legacy_id, "50000000-0000-4000-8000-000000000001"),
        )
    for space_id, organization_id, creator_id, legacy_id in [
        (SPACE, ORG, CREATOR, LEGACY_SPACE),
        (FOREIGN_SPACE, FOREIGN_ORG, FOREIGN_OWNER, LEGACY_FOREIGN_SPACE),
    ]:
        connection.execute(
            """INSERT INTO spaces(
                 id,organization_id,created_by_user_id,name,is_public,created_at_ms,updated_at_ms
               ) VALUES(?,?,?,?,0,1,1)""",
            (space_id, organization_id, creator_id, "Space"),
        )
        connection.execute(
            """INSERT INTO legacy_library_space_aliases_v1(
                 legacy_space_id,space_id,provenance,created_at_ms
               ) VALUES(?,?,'cap_backfill',1)""",
            (legacy_id, space_id),
        )
    connection.execute(
        """INSERT INTO space_members(
             space_id,user_id,role,state,created_at_ms,updated_at_ms
           ) VALUES(?,?,'manager','active',1,1)""",
        (SPACE, ACTOR),
    )
    # The owner has an exact imported membership alias but no global alias;
    # the adapter promotes this exact ID into the global immutable projection.
    connection.execute(
        """INSERT INTO legacy_space_member_aliases_v1(
             mapped_member_id,legacy_member_id,legacy_user_id,space_id,user_id,created_at_ms
           ) VALUES(?,?,?,?,?,1)""",
        (
            "60000000-0000-4000-8000-000000000001",
            LEGACY_OWNER_MEMBER,
            LEGACY_OWNER,
            SPACE,
            OWNER,
        ),
    )
    # Occupy the creator's first deterministic native candidate. INSERT OR
    # IGNORE must not drift the existing mapping or disclose this other user.
    connection.execute(
        """INSERT INTO legacy_collaboration_user_aliases_v1(
             legacy_user_id,mapped_user_id,image_url,provenance,created_at_ms,refreshed_at_ms
           ) VALUES(?,?,NULL,'native_generated',1,1)""",
        (native_alias_candidate(CREATOR, 0), COLLISION_USER),
    )
    connection.commit()


def rows(
    connection: sqlite3.Connection, name: str, parameters: tuple[object, ...]
) -> list[sqlite3.Row]:
    return connection.execute(sql(name), parameters).fetchall()


def promote_alias(
    connection: sqlite3.Connection,
    user_id: str,
    membership_alias: str | None,
) -> str:
    existing = rows(connection, "user_alias_read.sql", (user_id,))
    if existing:
        return existing[0]["legacy_user_id"]
    candidates = (
        [(membership_alias, "membership_backfill")]
        if membership_alias is not None
        else [
            (native_alias_candidate(user_id, attempt), "native_generated")
            for attempt in range(8)
        ]
    )
    for candidate, provenance in candidates:
        connection.execute(
            sql("user_alias_insert.sql"),
            (candidate, user_id, provenance, 2),
        )
        existing = rows(connection, "user_alias_read.sql", (user_id,))
        if existing:
            return existing[0]["legacy_user_id"]
    raise AssertionError("bounded native alias collision retries exhausted")


def access(connection: sqlite3.Connection, legacy_space_id: str) -> list[sqlite3.Row]:
    return rows(
        connection,
        "access_read.sql",
        (ACTOR, ORG, LEGACY_ORG, legacy_space_id),
    )


def main() -> None:
    connection = database()
    seed(connection)

    principal = rows(connection, "principal_scope.sql", (ACTOR,))
    assert len(principal) == 1
    assert principal[0]["active_organization_id"] == ORG
    assert principal[0]["active_legacy_organization_id"] == LEGACY_ORG

    initial = access(connection, LEGACY_SPACE)
    assert len(initial) == 1
    assert initial[0]["legacy_organization_owner_id"] is None
    assert initial[0]["legacy_created_by_id"] is None
    assert initial[0]["membership_organization_owner_id"] == LEGACY_OWNER
    assert initial[0]["organization_member_role"] == "admin"
    assert initial[0]["space_member_role"] == "admin"
    assert initial[0]["actor_is_organization_owner"] == 0
    assert initial[0]["actor_is_space_creator"] == 0
    assert "substr(replace" not in sql("access_read.sql")

    assert promote_alias(connection, OWNER, LEGACY_OWNER) == LEGACY_OWNER
    creator_alias = promote_alias(connection, CREATOR, None)
    assert creator_alias == native_alias_candidate(CREATOR, 1)
    assert creator_alias != native_alias_candidate(CREATOR, 0)
    projected = access(connection, LEGACY_SPACE)
    assert projected[0]["legacy_organization_owner_id"] == LEGACY_OWNER
    assert projected[0]["legacy_created_by_id"] == creator_alias

    # A retry never changes either persisted wire identity.
    assert promote_alias(connection, OWNER, LEGACY_OWNER) == LEGACY_OWNER
    assert promote_alias(connection, CREATOR, None) == creator_alias
    assert connection.execute(
        "SELECT legacy_user_id FROM legacy_collaboration_user_aliases_v1 WHERE mapped_user_id=?",
        (COLLISION_USER,),
    ).fetchone()[0] == native_alias_candidate(CREATOR, 0)

    connection.execute(
        "UPDATE space_members SET role='viewer' WHERE space_id=? AND user_id=?",
        (SPACE, ACTOR),
    )
    assert access(connection, LEGACY_SPACE)[0]["space_member_role"] == "member"
    connection.execute(
        "UPDATE space_members SET role='contributor' WHERE space_id=? AND user_id=?",
        (SPACE, ACTOR),
    )
    assert access(connection, LEGACY_SPACE)[0]["space_member_role"] == "contributor"

    # Foreign and missing aliases, deleted spaces, and tombstoned organizations
    # all retain the source null/not-found projection without cross-tenant data.
    assert access(connection, LEGACY_FOREIGN_SPACE) == []
    assert access(connection, "7123456789abcde") == []
    connection.execute("UPDATE spaces SET deleted_at_ms=3 WHERE id=?", (SPACE,))
    assert access(connection, LEGACY_SPACE) == []
    connection.execute("UPDATE spaces SET deleted_at_ms=NULL WHERE id=?", (SPACE,))
    connection.execute(
        "UPDATE organizations SET status='tombstoned',tombstoned_at_ms=4 WHERE id=?",
        (ORG,),
    )
    assert access(connection, LEGACY_SPACE) == []
    assert rows(connection, "principal_scope.sql", (ACTOR,)) == []

    print(
        "Legacy space-authorization SQLite conformance passed: active-tenant alias scope, "
        "owner/creator persisted wire aliases, deterministic collision retry/non-drift, "
        "manager/viewer role translation, and missing/deleted/tombstoned non-disclosure."
    )


if __name__ == "__main__":
    main()
