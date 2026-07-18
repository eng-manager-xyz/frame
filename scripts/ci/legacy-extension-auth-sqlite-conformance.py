#!/usr/bin/env python3
"""Provider-free SQLite proof for Cap extension auth/bootstrap semantics."""

from __future__ import annotations

import hashlib
import sqlite3
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_extension_auth"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}
NOW = 1_700_000_000_000


def identifier(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def database() -> sqlite3.Connection:
    db = sqlite3.connect(":memory:")
    db.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        db.executescript(migration.read_text(encoding="utf-8"))
    return db


def user(db: sqlite3.Connection, number: int, email: str) -> str:
    user_id = identifier(number)
    db.execute(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
        (user_id, email, email.split("@", 1)[0], 1, 1),
    )
    return user_id


def organization(
    db: sqlite3.Connection,
    number: int,
    owner_id: str,
    name: str,
    *,
    owner_membership: bool = True,
) -> str:
    organization_id = identifier(number)
    db.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,created_at_ms,updated_at_ms
           ) VALUES(?,?,?,'active',1,1)""",
        (organization_id, owner_id, name),
    )
    if owner_membership:
        db.execute(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,created_at_ms,updated_at_ms
               ) VALUES(?,?,'owner','active',1,1)""",
            (organization_id, owner_id),
        )
    return organization_id


def mint(db: sqlite3.Connection, actor_id: str, now_ms: int, number: int) -> str:
    secret = str(uuid.UUID(int=number, version=4))
    digest = hashlib.sha256(secret.encode()).hexdigest()
    row_id = identifier(10_000 + number)
    operation_id = identifier(20_000 + number)
    db.execute("BEGIN")
    try:
        db.execute(SQL["mint_insert"], (row_id, actor_id, digest, now_ms))
        db.execute(SQL["mint_overflow_delete"], (row_id, actor_id, now_ms))
        db.execute(SQL["mint_assert"], (operation_id, row_id, actor_id, digest))
        db.execute(SQL["assertion_delete"], (operation_id,))
        db.commit()
    except Exception:
        db.rollback()
        raise
    return secret


def repair(
    db: sqlite3.Connection,
    actor_id: str,
    organization_id: str,
    number: int,
) -> None:
    operation_id = identifier(30_000 + number)
    db.commit()
    db.execute("BEGIN")
    try:
        db.execute(
            SQL["bootstrap_repair"],
            (actor_id, organization_id, operation_id, NOW),
        )
        db.execute(
            SQL["bootstrap_repair_assert"],
            (operation_id, actor_id, organization_id),
        )
        db.execute(SQL["assertion_delete"], (operation_id,))
        db.commit()
    except Exception:
        db.rollback()
        raise


def is_pro(status: str | None, third_party: str | None, *, is_cap: bool) -> bool:
    return not is_cap or bool(third_party) or status in {
        "active",
        "trialing",
        "complete",
        "paid",
    }


def main() -> None:
    expected_queries = {
        "api_key_actor",
        "assertion_delete",
        "bootstrap_repair",
        "bootstrap_repair_assert",
        "bootstrap_resolve",
        "mint_assert",
        "mint_insert",
        "mint_overflow_delete",
        "mint_recent_count",
        "revoke_owned",
        "session_user",
    }
    assert set(SQL) == expected_queries
    db = database()

    actor = user(db, 1, "actor@example.test")
    other = user(db, 2, "other@example.test")
    cross = organization(db, 100, other, "Cross tenant")
    owned_z = organization(db, 102, actor, "Owned Z")
    owned_a = organization(db, 101, actor, "Owned A")
    db.execute(
        "UPDATE users SET active_organization_id=? WHERE id=?", (cross, actor)
    )
    db.commit()

    # Insert-count-delete-assert is one transaction. Ten keys settle; the
    # eleventh is deleted before the assertion aborts and leaves no residue.
    secrets = [mint(db, actor, NOW, index + 1) for index in range(10)]
    assert all(len(secret) == 36 for secret in secrets)
    assert db.execute(
        "SELECT COUNT(*) FROM auth_api_keys WHERE user_id=?", (actor,)
    ).fetchone()[0] == 10
    try:
        mint(db, actor, NOW, 11)
        raise AssertionError("eleventh hourly extension key was minted")
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_extension_auth_assertion_failed_v1" in str(error)
    assert db.execute(
        "SELECT COUNT(*) FROM auth_api_keys WHERE user_id=?", (actor,)
    ).fetchone()[0] == 10
    assert db.execute(
        "SELECT COUNT(*) FROM legacy_extension_auth_assertions_v1"
    ).fetchone()[0] == 0

    # Source uses a strict `createdAt > now - one hour` window; rows exactly
    # on the boundary do not count.
    boundary_secret = mint(db, actor, NOW + 3_600_000, 12)
    assert len(boundary_secret) == 36

    # API-key middleware derives the actor from the digest, rejects expiry,
    # and revoke is constrained by both the resolved actor and key digest.
    boundary_digest = hashlib.sha256(boundary_secret.encode()).hexdigest()
    assert tuple(
        db.execute(SQL["api_key_actor"], (boundary_digest, NOW)).fetchone()
    ) == (actor, "actor@example.test")
    db.execute(SQL["revoke_owned"], (other, boundary_digest))
    assert db.execute(
        "SELECT 1 FROM auth_api_keys WHERE key_digest=?", (boundary_digest,)
    ).fetchone()
    db.execute(SQL["revoke_owned"], (actor, boundary_digest))
    assert db.execute(
        "SELECT 1 FROM auth_api_keys WHERE key_digest=?", (boundary_digest,)
    ).fetchone() is None

    # A corrupt active-pointer cannot grant a cross-tenant active-org result.
    # Deterministic owned fallback chooses the lowest organization id, repairs
    # the pointer, and bumps Frame's selection revision.
    selected = db.execute(SQL["bootstrap_resolve"], (actor,)).fetchone()
    assert selected["id"] == owned_a
    assert selected["name"] == "Owned A"
    assert selected["active_organization_id"] == cross
    repair(db, actor, owned_a, 1)
    assert tuple(
        db.execute(
            "SELECT active_organization_id,organization_preference_revision FROM users WHERE id=?",
            (actor,),
        ).fetchone()
    ) == (owned_a, 1)

    # Active owned wins even when another owned ID sorts first.
    db.execute("UPDATE users SET active_organization_id=? WHERE id=?", (owned_z, actor))
    assert db.execute(SQL["bootstrap_resolve"], (actor,)).fetchone()["id"] == owned_z

    # A user with no owned organization falls back to the oldest live
    # membership, with organization ID as the stable tie-break.
    member = user(db, 3, "member@example.test")
    older = organization(db, 201, other, "Older", owner_membership=False)
    newer = organization(db, 200, other, "Newer", owner_membership=False)
    db.executemany(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,created_at_ms,updated_at_ms
           ) VALUES(?,?,'member','active',?,?)""",
        [(newer, member, 20, 20), (older, member, 10, 10)],
    )
    member_choice = db.execute(SQL["bootstrap_resolve"], (member,)).fetchone()
    assert member_choice["id"] == older
    repair(db, member, older, 2)

    # Subscription fields belong to the selected organization's owner.
    db.execute(
        "UPDATE users SET legacy_stripe_subscription_status='canceled', legacy_third_party_stripe_subscription_id='third-party' WHERE id=?",
        (other,),
    )
    entitled = db.execute(SQL["bootstrap_resolve"], (member,)).fetchone()
    assert is_pro(
        entitled["stripe_subscription_status"],
        entitled["third_party_stripe_subscription_id"],
        is_cap=True,
    )
    assert is_pro("canceled", None, is_cap=False)
    assert not is_pro("canceled", None, is_cap=True)

    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
    print(
        "legacy extension auth SQLite conformance passed: digest-only UUID keys, "
        "atomic hourly cap, actor-owned revoke, deterministic bootstrap repair, "
        "owner entitlement, and non-Cap unlimited branch"
    )


if __name__ == "__main__":
    main()
