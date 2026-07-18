#!/usr/bin/env python3
"""SQLite proof for the four source-pinned Cap mobile session routes."""

from __future__ import annotations

import hashlib
import sqlite3
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_mobile_session"
FIXTURE = ROOT / "fixtures/api-parity/v1/mobile-session.json"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}
NOW = 1_700_000_000_000
ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"


def uid(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def cap_id(number: int) -> str:
    value = []
    for _ in range(15):
        value.append(ALPHABET[number & 31])
        number >>= 5
    return "".join(reversed(value))


def mapped(value: str) -> str:
    digest = hashlib.sha256(b"frame-cap-nanoid-to-uuid-v1\0" + value.encode()).digest()
    data = bytearray(digest[:16])
    data[6] = (data[6] & 0x0F) | 0x80
    data[8] = (data[8] & 0x3F) | 0x80
    return str(uuid.UUID(bytes=bytes(data)))


def digest(value: str | bytes) -> str:
    if isinstance(value, str):
        value = value.encode()
    return hashlib.sha256(value).hexdigest()


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:", isolation_level=None)
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def user(connection: sqlite3.Connection, number: int, email: str) -> tuple[str, str]:
    legacy = cap_id(number)
    mapped_id = mapped(legacy)
    connection.execute(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?)",
        (mapped_id, email, None, 1, 1),
    )
    connection.execute(
        """INSERT INTO legacy_collaboration_user_aliases_v1(
             legacy_user_id,mapped_user_id,image_url,provenance,created_at_ms,refreshed_at_ms
           ) VALUES(?,?,NULL,'native_generated',1,1)""",
        (legacy, mapped_id),
    )
    return mapped_id, legacy


def organization(
    connection: sqlite3.Connection, number: int, owner_id: str
) -> tuple[str, str]:
    legacy = cap_id(10_000 + number)
    mapped_id = mapped(legacy)
    connection.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
           ) VALUES(?,?,'Existing Organization','active','{}',1,1)""",
        (mapped_id, owner_id),
    )
    connection.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,'owner','active',0,1,1)""",
        (mapped_id, owner_id),
    )
    return mapped_id, legacy


def pending_invite(
    connection: sqlite3.Connection,
    number: int,
    organization_id: str,
    owner_id: str,
    email: str,
) -> str:
    legacy = cap_id(20_000 + number)
    mapped_id = mapped(legacy)
    connection.execute(
        """INSERT INTO organization_invites(
             id,organization_id,invited_email_digest,invited_by_user_id,role,status,
             token_digest,created_at_ms,expires_at_ms
           ) VALUES(?,?,?,?,'member','pending',?,1,9007199254740991)""",
        (mapped_id, organization_id, digest(email.lower()), owner_id, digest(f"token-{number}")),
    )
    connection.execute(
        """INSERT INTO legacy_invite_lifecycle_invite_aliases_v1(
             mapped_invite_id,legacy_invite_id,organization_id,invited_email,
             legacy_role,decision,recorded_at_ms
           ) VALUES(?,?,?,?, 'member','pending',1)""",
        (mapped_id, legacy, organization_id, email),
    )
    return legacy


def operation(
    connection: sqlite3.Connection,
    operation_id: str,
    action: str,
    actor_id: str | None,
    subject: str,
    provider_effect: str,
    outcome: str,
    legacy_user_id: str | None = None,
    key_row_id: str | None = None,
    delivery_id: str | None = None,
    affected: int = 0,
) -> None:
    connection.execute(
        SQL["operation_insert"],
        (operation_id, action, actor_id, subject, provider_effect, NOW),
    )
    connection.execute(
        SQL["receipt_insert"],
        (
            operation_id,
            outcome,
            actor_id,
            legacy_user_id,
            key_row_id,
            delivery_id,
            affected,
            NOW,
        ),
    )
    connection.execute(
        SQL["audit_insert"],
        (uid(int(operation_id[-4:], 16) + 30_000), operation_id, actor_id, action, subject, NOW),
    )
    connection.execute(
        SQL["postcondition_assert"],
        (operation_id, "operation_receipt_audit", 1, action, outcome),
    )


def prove_request_replace_and_ciphertext_only(connection: sqlite3.Connection) -> None:
    email = "new.user@example.com"
    identifier = digest(email)
    code = "123456"
    token = digest(code + "nextauth-secret")
    payload = bytes([0xA5]) * 1_071
    payload_hex = payload.hex()
    delivery_one = uid(100)
    operation_one = uid(101)
    connection.execute("BEGIN")
    connection.execute(SQL["handoff_insert"], (delivery_one, payload_hex, digest(payload), NOW))
    connection.execute(
        SQL["challenge_upsert"], (identifier, token, delivery_one, NOW, operation_one)
    )
    operation(
        connection,
        operation_one,
        "email_request",
        None,
        identifier,
        "email_handoff_pending",
        "challenge_replaced",
        delivery_id=delivery_one,
    )
    connection.execute(
        SQL["challenge_postcondition_assert"],
        (operation_one, identifier, token, delivery_one, NOW),
    )
    connection.execute(SQL["assertion_cleanup"], (operation_one,))
    connection.execute("COMMIT")

    replacement_code = "654321"
    replacement = digest(replacement_code + "nextauth-secret")
    delivery_two = uid(102)
    operation_two = uid(103)
    connection.execute("BEGIN")
    connection.execute(SQL["handoff_insert"], (delivery_two, payload_hex, digest(payload), NOW + 1))
    connection.execute(
        SQL["challenge_upsert"],
        (identifier, replacement, delivery_two, NOW + 1, operation_two),
    )
    operation(
        connection,
        operation_two,
        "email_request",
        None,
        identifier,
        "email_handoff_pending",
        "challenge_replaced",
        delivery_id=delivery_two,
    )
    connection.execute(SQL["assertion_cleanup"], (operation_two,))
    connection.execute("COMMIT")

    row = connection.execute(SQL["challenge_snapshot"], (identifier,)).fetchone()
    assert row["token_digest"] == replacement
    assert row["expires_at_ms"] == NOW + 1 + 600_000
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_delivery_provider_handoffs_v1"
    ).fetchone()[0] == 2
    persisted = connection.execute(
        "SELECT payload_hex FROM auth_delivery_provider_handoffs_v1 WHERE delivery_id=?",
        (delivery_two,),
    ).fetchone()[0]
    assert len(persisted) == 2_142
    assert email.encode().hex() not in persisted
    assert replacement_code.encode().hex() not in persisted


def prove_destructive_one_use(connection: sqlite3.Connection) -> None:
    identifier = digest("attempt@example.com")
    payload = bytes([7]) * 1_071

    def challenge(number: int, token: str, created: int) -> None:
        delivery = uid(200 + number)
        op = uid(300 + number)
        connection.execute(SQL["handoff_insert"], (delivery, payload.hex(), digest(payload), created))
        connection.execute(SQL["challenge_upsert"], (identifier, token, delivery, created, op))

    challenge(1, digest("correctsecret"), NOW)
    connection.execute(SQL["challenge_delete_identifier"], (identifier,))
    assert connection.execute(SQL["challenge_snapshot"], (identifier,)).fetchone() is None

    challenge(2, digest("correctsecret"), NOW - 600_001)
    expired = connection.execute(SQL["challenge_snapshot"], (identifier,)).fetchone()
    assert expired["expires_at_ms"] < NOW
    connection.execute(SQL["challenge_delete_identifier"], (identifier,))

    correct = digest("correctsecret")
    challenge(3, correct, NOW)
    assert connection.execute(SQL["challenge_delete_matching"], (identifier, correct)).rowcount == 1
    assert connection.execute(SQL["challenge_delete_matching"], (identifier, correct)).rowcount == 0


def prove_user_adapter_branches(connection: sqlite3.Connection) -> None:
    visible_id, visible_legacy = user(connection, 1, "visible@example.com")
    connection.execute(
        """INSERT INTO identity_accounts(
             id,user_id,provider,provider_account_id,created_at_ms,updated_at_ms
           ) VALUES(?,?, 'google','google-visible',1,1)""",
        (uid(400), visible_id),
    )
    visible = connection.execute(SQL["user_snapshot"], ("visible@example.com",)).fetchone()
    assert visible["has_linked_account"] == 1
    assert visible["has_pending_provisioned_invite"] == 0
    connection.execute(SQL["user_verify_visible"], (visible_id, NOW))
    assert connection.execute(
        "SELECT email_verified_at_ms FROM users WHERE id=?", (visible_id,)
    ).fetchone()[0] == NOW
    assert connection.execute(
        "SELECT COUNT(*) FROM legacy_mobile_session_stripe_effects_v1 WHERE user_id=?",
        (visible_id,),
    ).fetchone()[0] == 0

    owner_id, _ = user(connection, 2, "owner@example.com")
    org_id, _ = organization(connection, 1, owner_id)
    hidden_id, hidden_legacy = user(connection, 3, "first._-last@example.com")
    connection.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,'member','active',0,1,1)""",
        (org_id, hidden_id),
    )
    pending_invite(connection, 1, org_id, owner_id, "first._-last@example.com")
    hidden = connection.execute(
        SQL["user_snapshot"], ("first._-last@example.com",)
    ).fetchone()
    assert hidden["has_linked_account"] == 0
    assert hidden["has_pending_provisioned_invite"] == 1
    hidden_operation = uid(401)
    hidden_subject = digest("first._-last@example.com")
    connection.execute("BEGIN")
    connection.execute(
        SQL["user_authority_assert"],
        (hidden_operation, hidden_id, "first._-last@example.com", hidden_legacy),
    )
    connection.execute(SQL["user_verify_provisioned"], (hidden_id, "first last", NOW))
    operation(
        connection,
        hidden_operation,
        "email_verify",
        hidden_id,
        hidden_subject,
        "stripe_sync_pending",
        "user_provisioned_provider_pending",
        legacy_user_id=hidden_legacy,
    )
    connection.execute(
        SQL["stripe_effect_insert"],
        (uid(402), hidden_operation, hidden_id, hidden_subject, NOW),
    )
    connection.execute(
        SQL["stripe_effect_postcondition_assert"],
        (hidden_operation, hidden_id, hidden_subject),
    )
    connection.execute(SQL["assertion_cleanup"], (hidden_operation,))
    connection.execute("COMMIT")
    updated = connection.execute(
        "SELECT display_name,email_verified_at_ms FROM users WHERE id=?", (hidden_id,)
    ).fetchone()
    assert tuple(updated) == ("first last", NOW)
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_api_keys WHERE user_id=? AND legacy_source='mobile'",
        (hidden_id,),
    ).fetchone()[0] == 0

    invited_email = "invited-new@example.com"
    pending_invite(connection, 2, org_id, owner_id, invited_email)
    assert connection.execute(SQL["pending_invite"], (invited_email,)).fetchone()[0] == 1
    new_legacy = cap_id(4)
    new_id = mapped(new_legacy)
    connection.execute(SQL["user_insert"], (new_id, invited_email, NOW))
    connection.execute(SQL["user_alias_insert"], (new_legacy, new_id, NOW))
    assert tuple(connection.execute(
        "SELECT active_organization_id,default_organization_id FROM users WHERE id=?", (new_id,)
    ).fetchone()) == (None, None)

    ordinary_email = "ordinary-new@example.com"
    assert connection.execute(SQL["pending_invite"], (ordinary_email,)).fetchone()[0] == 0
    ordinary_legacy = cap_id(5)
    ordinary_id = mapped(ordinary_legacy)
    ordinary_org_legacy = cap_id(6)
    ordinary_org = mapped(ordinary_org_legacy)
    ordinary_member_legacy = cap_id(7)
    ordinary_member = mapped(ordinary_member_legacy)
    operation_id = uid(403)
    connection.execute("BEGIN")
    connection.execute(SQL["user_insert"], (ordinary_id, ordinary_email, NOW))
    connection.execute(SQL["user_alias_insert"], (ordinary_legacy, ordinary_id, NOW))
    connection.execute(SQL["organization_insert"], (ordinary_org, ordinary_id, NOW))
    connection.execute(
        SQL["organization_alias_insert"],
        (ordinary_org, ordinary_org_legacy, NOW, operation_id),
    )
    connection.execute(SQL["member_insert"], (ordinary_org, ordinary_id, NOW))
    connection.execute(
        SQL["member_alias_insert"],
        (ordinary_member, ordinary_member_legacy, ordinary_org, ordinary_id, NOW, operation_id),
    )
    connection.execute(
        SQL["user_organization_select"], (ordinary_id, ordinary_org, operation_id, NOW)
    )
    connection.execute("COMMIT")
    ordinary = connection.execute(
        "SELECT active_organization_id,default_organization_id FROM users WHERE id=?",
        (ordinary_id,),
    ).fetchone()
    assert tuple(ordinary) == (ordinary_org, ordinary_org)
    assert visible_legacy and hidden_legacy


def prove_replace_all_revoke_and_rollback(connection: sqlite3.Connection) -> None:
    actor_id, legacy_actor = user(connection, 8, "keys@example.com")
    for number, source in [(500, "mobile"), (501, "mobile"), (502, "extension")]:
        connection.execute(
            """INSERT INTO auth_api_keys(
                 id,user_id,key_digest,name,scopes_json,created_at_ms,legacy_source
               ) VALUES(?,?,?,'old','[]',1,?)""",
            (uid(number), actor_id, digest(f"old-{number}"), source),
        )
    new_raw = "00000000-0000-4000-8000-000000000123"
    new_row = uid(503)
    new_digest = digest(new_raw)
    operation_id = uid(504)
    connection.execute("BEGIN")
    connection.execute(SQL["mobile_keys_delete"], (actor_id,))
    connection.execute(SQL["mobile_key_insert"], (new_row, actor_id, new_digest, NOW))
    operation(
        connection,
        operation_id,
        "session_request",
        actor_id,
        digest(legacy_actor),
        "not_requested",
        "api_key_replaced",
        legacy_user_id=legacy_actor,
        key_row_id=new_row,
        affected=3,
    )
    connection.execute(
        SQL["mobile_key_postcondition_assert"],
        (operation_id, new_row, actor_id, new_digest),
    )
    connection.execute(SQL["assertion_cleanup"], (operation_id,))
    connection.execute("COMMIT")
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_api_keys WHERE user_id=? AND legacy_source='mobile'",
        (actor_id,),
    ).fetchone()[0] == 1
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_api_keys WHERE user_id=? AND legacy_source='extension'",
        (actor_id,),
    ).fetchone()[0] == 1

    # A failed checked assertion rolls the complete replacement back.
    try:
        connection.execute("BEGIN")
        connection.execute(SQL["mobile_keys_delete"], (actor_id,))
        connection.execute(SQL["mobile_key_insert"], (uid(505), actor_id, digest("other"), NOW))
        connection.execute(
            SQL["mobile_key_postcondition_assert"],
            (uid(506), uid(505), actor_id, digest("wrong")),
        )
        raise AssertionError("mismatched key assertion unexpectedly committed")
    except sqlite3.IntegrityError:
        connection.execute("ROLLBACK")
    assert connection.execute(
        "SELECT key_digest FROM auth_api_keys WHERE id=?", (new_row,)
    ).fetchone()[0] == new_digest

    revoke_operation = uid(507)
    connection.execute("BEGIN")
    connection.execute(SQL["mobile_key_revoke"], (new_digest,))
    operation(
        connection,
        revoke_operation,
        "session_revoke",
        actor_id,
        digest(legacy_actor),
        "not_requested",
        "api_key_revoked",
        legacy_user_id=legacy_actor,
        affected=1,
    )
    connection.execute(SQL["revoke_postcondition_assert"], (revoke_operation, new_digest))
    connection.execute(SQL["assertion_cleanup"], (revoke_operation,))
    connection.execute("COMMIT")
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_api_keys WHERE key_digest=?", (new_digest,)
    ).fetchone()[0] == 0


def main() -> None:
    assert FIXTURE.exists(), "mobile-session fixture is missing"
    expected_queries = {
        "challenge_upsert",
        "challenge_delete_matching",
        "user_snapshot",
        "mobile_keys_delete",
        "stripe_effect_insert",
        "revoke_postcondition_assert",
    }
    assert expected_queries <= SQL.keys(), "checked SQL closure is incomplete"
    connection = database()
    prove_request_replace_and_ciphertext_only(connection)
    prove_destructive_one_use(connection)
    prove_user_adapter_branches(connection)
    prove_replace_all_revoke_and_rollback(connection)
    foreign_key_check = connection.execute("PRAGMA foreign_key_check").fetchall()
    assert foreign_key_check == [], foreign_key_check
    print(
        "legacy mobile session SQLite conformance passed: replacement, one-use deletion, "
        "adapter/Stripe branches, Cap-ID provisioning, replace-all keys, revoke, and rollback"
    )


if __name__ == "__main__":
    main()
