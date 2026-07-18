#!/usr/bin/env python3
"""SQLite proof for source-pinned invite accept/decline transactions."""

from __future__ import annotations

import hashlib
import sqlite3
import uuid
from pathlib import Path
from typing import Callable

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_invite_lifecycle"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}
NOW = 1_700_000_000_000
ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"


def uid(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def cap_id(number: int) -> str:
    result = []
    for _ in range(15):
        result.append(ALPHABET[number & 31])
        number >>= 5
    return "".join(reversed(result))


def mapped(value: str) -> str:
    digest = hashlib.sha256(b"frame-cap-nanoid-to-uuid-v1\0" + value.encode()).digest()
    data = bytearray(digest[:16])
    data[6] = (data[6] & 0x0F) | 0x80
    data[8] = (data[8] & 0x3F) | 0x80
    return str(uuid.UUID(bytes=bytes(data)))


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:", isolation_level=None)
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def user(connection: sqlite3.Connection, number: int, email: str) -> str:
    user_id = uid(number)
    connection.execute(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
        (user_id, email, email.split("@")[0], 1, 1),
    )
    return user_id


def organization(connection: sqlite3.Connection, number: int, owner: str, quota: int = 3) -> str:
    organization_id = uid(100 + number)
    connection.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
           ) VALUES(?,?,?,'active','{}',1,1)""",
        (organization_id, owner, f"Organization {number}"),
    )
    connection.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,'owner','active',0,1,1)""",
        (organization_id, owner),
    )
    connection.execute(
        """UPDATE users SET legacy_invite_quota=?, legacy_stripe_subscription_id='sub_owner'
           WHERE id=?""",
        (quota, owner),
    )
    return organization_id


def invite(
    connection: sqlite3.Connection,
    number: int,
    organization_id: str,
    owner: str,
    email: str,
    role: str,
) -> tuple[str, str]:
    legacy_id = cap_id(1000 + number)
    mapped_id = mapped(legacy_id)
    connection.execute(
        """INSERT INTO organization_invites(
             id,organization_id,invited_email_digest,invited_by_user_id,role,status,
             token_digest,created_at_ms,expires_at_ms
           ) VALUES(?,?,?,?,'member','pending',?,1,9999999999999)""",
        (mapped_id, organization_id, hashlib.sha256(email.lower().encode()).hexdigest(), owner,
         hashlib.sha256(f"token-{number}".encode()).hexdigest()),
    )
    connection.execute(
        """INSERT INTO legacy_invite_lifecycle_invite_aliases_v1(
             mapped_invite_id,legacy_invite_id,organization_id,invited_email,
             legacy_role,decision,recorded_at_ms
           ) VALUES(?,?,?,?,?,'pending',1)""",
        (mapped_id, legacy_id, organization_id, email, role),
    )
    return legacy_id, mapped_id


def alias(connection: sqlite3.Connection, organization_id: str, actor: str, number: int) -> None:
    legacy_id = cap_id(5000 + number)
    connection.execute(
        """INSERT INTO legacy_invite_lifecycle_member_aliases_v1(
             mapped_member_id,legacy_member_id,organization_id,user_id,created_at_ms
           ) VALUES(?,?,?,?,1)""",
        (mapped(legacy_id), legacy_id, organization_id, actor),
    )


def decide(
    connection: sqlite3.Connection,
    actor: str,
    legacy_invite: str,
    action: str,
    before_batch: Callable[[sqlite3.Connection, sqlite3.Row], None] | None = None,
) -> sqlite3.Row:
    snapshot = connection.execute(SQL["snapshot"], (actor, legacy_invite)).fetchone()
    if snapshot is None:
        raise LookupError("Invite not found")
    if snapshot["actor_email"].lower() != snapshot["invited_email"].lower():
        raise PermissionError("Email mismatch")
    operation_id = uid(9000 + decide.serial)
    decide.serial += 1
    exists = snapshot["membership_exists"]
    created = action == "accept" and not exists
    removed = action == "decline" and bool(exists)
    subscription = snapshot["owner_subscription_id"]
    pro = bool(created and subscription and snapshot["owner_invite_quota"] - snapshot["pro_seats_used"] > 0)
    cleared = bool(removed and snapshot["membership_has_pro_seat"] and not snapshot["other_pro_seat_count"])
    decision = f"{action}ed" if action == "accept" else "declined"
    actor_id = actor
    if before_batch is not None:
        before_batch(connection, snapshot)
    connection.execute("BEGIN")
    try:
        connection.execute(SQL["operation_insert"], (operation_id, actor_id, snapshot["organization_id"], legacy_invite, action, NOW))
        connection.execute(SQL["authority_assert"], (
            operation_id, actor_id, snapshot["organization_id"], legacy_invite,
            snapshot["mapped_invite_id"], snapshot["invited_email"], snapshot["legacy_role"], snapshot["actor_email"],
            snapshot["owner_id"], snapshot["owner_invite_quota"], snapshot["owner_subscription_id"],
            snapshot["membership_exists"], snapshot["membership_has_pro_seat"],
            snapshot["mapped_member_id"], snapshot["legacy_member_id"], snapshot["pro_seats_used"],
            snapshot["fallback_organization_id"], snapshot["other_pro_seat_count"],
        ))
        if action == "accept":
            role = "admin" if snapshot["legacy_role"].lower() == "admin" else "member"
            new_legacy = cap_id(8000 + decide.serial)
            connection.execute(SQL["accept_membership_insert"], (snapshot["organization_id"], actor_id, role, NOW, operation_id, exists))
            connection.execute(SQL["accept_member_alias_insert"], (mapped(new_legacy), new_legacy, snapshot["organization_id"], actor_id, NOW, operation_id, exists))
            connection.execute(SQL["accept_pro_seat_update"], (snapshot["organization_id"], actor_id, pro, NOW, operation_id))
            connection.execute(SQL["accept_user_update"], (actor_id, snapshot["organization_id"], pro, subscription, NOW))
        else:
            fallback = snapshot["fallback_organization_id"]
            connection.execute(SQL["decline_space_members_delete"], (actor_id, snapshot["organization_id"], removed))
            connection.execute(SQL["decline_member_alias_update"], (snapshot["organization_id"], actor_id, NOW, operation_id, removed))
            connection.execute(SQL["decline_membership_delete"], (snapshot["organization_id"], actor_id, removed))
            connection.execute(SQL["decline_user_update"], (actor_id, snapshot["organization_id"], fallback, cleared, NOW, removed))
        connection.execute(SQL["invite_delete"], (snapshot["mapped_invite_id"], snapshot["organization_id"]))
        connection.execute(SQL["invite_alias_resolve"], (snapshot["mapped_invite_id"], decision, NOW, operation_id))
        connection.execute(SQL["receipt_insert"], (
            operation_id, action, exists, created, removed, pro, cleared,
            snapshot["fallback_organization_id"], NOW,
        ))
        connection.execute(SQL["audit_insert"], (operation_id, actor_id, snapshot["organization_id"], action, NOW))
        connection.execute(SQL["operation_complete"], (operation_id, NOW))
        connection.execute(SQL["postcondition_assert"], (operation_id, actor_id, snapshot["organization_id"], snapshot["mapped_invite_id"], decision, action))
        connection.execute(SQL["assertion_cleanup"], (operation_id,))
        connection.execute("COMMIT")
    except Exception:
        connection.execute("ROLLBACK")
        raise
    return connection.execute(
        "SELECT * FROM legacy_invite_lifecycle_receipts_v1 WHERE operation_id=?", (operation_id,)
    ).fetchone()


decide.serial = 1


def main() -> None:
    db = database()
    owner = user(db, 1, "owner@example.test")
    actor = user(db, 2, "Invitee@Example.Test")
    org = organization(db, 1, owner, quota=3)

    # New membership: invalid/owner-like source roles normalize to member,
    # case-insensitive email matches, a seat is inherited, and onboarding merges.
    db.execute("UPDATE users SET legacy_onboarding_steps_json='{" + '"welcome":true' + "}' WHERE id=?", (actor,))
    accept_id, accept_mapped = invite(db, 1, org, owner, "invitee@example.test", "OWNER")
    try:
        db.execute(
            "UPDATE legacy_invite_lifecycle_invite_aliases_v1 SET invited_email=? WHERE mapped_invite_id=?",
            ("INVITEE@EXAMPLE.TEST", accept_mapped),
        )
        raise AssertionError("case-only alias mutation accepted")
    except sqlite3.IntegrityError:
        pass
    receipt = decide(db, actor, accept_id, "accept")
    assert receipt["membership_created"] == 1 and receipt["pro_seat_assigned"] == 1
    member = db.execute("SELECT role,has_pro_seat FROM organization_members WHERE organization_id=? AND user_id=?", (org, actor)).fetchone()
    assert tuple(member) == ("member", 1)
    updated = db.execute("SELECT active_organization_id,default_organization_id,legacy_onboarding_steps_json,legacy_third_party_stripe_subscription_id FROM users WHERE id=?", (actor,)).fetchone()
    assert updated[0] == org and updated[1] == org and updated[3] == "sub_owner"
    assert all(f'"{name}":true' in updated[2] for name in ("welcome", "organizationSetup", "customDomain", "inviteTeam"))
    assert db.execute("SELECT 1 FROM organization_invites WHERE id=?", (accept_mapped,)).fetchone() is None
    assert db.execute("SELECT decision FROM legacy_invite_lifecycle_invite_aliases_v1 WHERE mapped_invite_id=?", (accept_mapped,)).fetchone()[0] == "accepted"

    # Existing membership path does not allocate another alias or overwrite role.
    existing_id, _ = invite(db, 2, org, owner, "INVITEE@EXAMPLE.TEST", "ADMIN")
    alias_count = db.execute("SELECT COUNT(*) FROM legacy_invite_lifecycle_member_aliases_v1 WHERE user_id=?", (actor,)).fetchone()[0]
    existing = decide(db, actor, existing_id, "accept")
    assert existing["membership_created"] == 0 and existing["pro_seat_assigned"] == 0
    assert db.execute("SELECT COUNT(*) FROM legacy_invite_lifecycle_member_aliases_v1 WHERE user_id=?", (actor,)).fetchone()[0] == alias_count

    # Decline removes every organization-space membership, repairs pointers to
    # the deterministic remaining organization, and clears inherited billing.
    fallback_owner = user(db, 3, "fallback-owner@example.test")
    fallback = organization(db, 2, fallback_owner)
    db.execute("INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms) VALUES(?,?,'member','active',0,2,2)", (fallback, actor))
    alias(db, fallback, actor, 3)
    space = uid(500)
    db.execute("INSERT INTO spaces(id,organization_id,created_by_user_id,name,created_at_ms,updated_at_ms) VALUES(?,?,?,'Team',1,1)", (space, org, owner))
    db.execute("INSERT INTO space_members(space_id,user_id,role,created_at_ms,updated_at_ms,state) VALUES(?,?,'viewer',1,1,'active')", (space, actor))
    decline_id, _ = invite(db, 3, org, owner, "invitee@example.test", "member")
    declined = decide(db, actor, decline_id, "decline")
    assert declined["membership_removed"] == 1 and declined["inherited_subscription_cleared"] == 1
    assert db.execute("SELECT 1 FROM organization_members WHERE organization_id=? AND user_id=?", (org, actor)).fetchone() is None
    assert db.execute("SELECT 1 FROM space_members WHERE space_id=? AND user_id=?", (space, actor)).fetchone() is None
    pointers = db.execute("SELECT active_organization_id,default_organization_id,legacy_third_party_stripe_subscription_id FROM users WHERE id=?", (actor,)).fetchone()
    assert tuple(pointers) == (fallback, fallback, None)

    # Declining without a membership deletes only the invite and leaves pointers.
    no_member_id, _ = invite(db, 4, org, owner, "invitee@example.test", "member")
    no_member = decide(db, actor, no_member_id, "decline")
    assert no_member["membership_removed"] == 0
    assert db.execute("SELECT active_organization_id FROM users WHERE id=?", (actor,)).fetchone()[0] == fallback

    # Not-found and mismatch paths have no mutation.
    try:
        decide(db, actor, cap_id(99999), "accept")
        raise AssertionError("missing invite accepted")
    except LookupError:
        pass
    mismatch, mismatch_mapped = invite(db, 5, org, owner, "other@example.test", "member")
    try:
        decide(db, actor, mismatch, "accept")
        raise AssertionError("email mismatch accepted")
    except PermissionError:
        pass
    assert db.execute("SELECT 1 FROM organization_invites WHERE id=?", (mismatch_mapped,)).fetchone()

    # Every decision input read before the D1 batch is captured in its first
    # assertion. A concurrent quota change therefore fails closed before any
    # membership, pointer, invite, receipt, or audit mutation can commit.
    stale_id, stale_mapped = invite(db, 6, org, owner, "invitee@example.test", "member")
    try:
        decide(
            db,
            actor,
            stale_id,
            "accept",
            lambda connection, _snapshot: connection.execute(
                "UPDATE users SET legacy_invite_quota=legacy_invite_quota+1 WHERE id=?",
                (owner,),
            ),
        )
        raise AssertionError("stale authority accepted")
    except sqlite3.IntegrityError:
        pass
    assert db.execute("SELECT 1 FROM organization_invites WHERE id=?", (stale_mapped,)).fetchone()
    assert db.execute(
        "SELECT COUNT(*) FROM legacy_invite_lifecycle_operations_v1 WHERE legacy_invite_id=?",
        (stale_id,),
    ).fetchone()[0] == 0

    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
    print("legacy invite lifecycle SQLite conformance passed: accept/decline, aliases, seats, onboarding, fallback, cleanup, stale-authority rollback, failures")


if __name__ == "__main__":
    main()
