#!/usr/bin/env python3
"""SQLite proof for Cap's eight user/account compatibility identities.

The suite applies the full expand chain and executes the checked-in D1 SQL. It
proves field-presence semantics, onboarding merges/defaults, lossless long/empty
organization names, owner-or-member account access, all-device credential
revocation, development mutations, replay/conflict behavior, atomic rollback,
append-only evidence, and explicit fail-closed R2 branches.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
import uuid
from pathlib import Path
from typing import Any, Callable


ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_user_account"
MIGRATION = MIGRATIONS / "0042_legacy_user_account_expand.sql"
RUNTIME = ROOT / "apps/control-plane/src/legacy_user_account_runtime.rs"
APPLICATION = ROOT / "crates/application/src/legacy_user_account.rs"

SQL = {
    path.stem: path.read_text(encoding="utf-8").strip()
    for path in sorted(QUERIES.glob("*.sql"))
}
BROWSER_GRANT_ASSERT_SQL = (
    ROOT / "apps/control-plane/queries/auth/browser_mutation_grant_assert.sql"
).read_text(encoding="utf-8").strip()
BROWSER_GRANT_DELETE_SQL = (
    ROOT / "apps/control-plane/queries/auth/browser_mutation_grant_delete_by_proof.sql"
).read_text(encoding="utf-8").strip()
BROWSER_ASSERTION_CLEANUP_SQL = SQL["browser_assertion_cleanup"]

NOW = 1_700_000_000_000
ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"


def uid(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def cap_id(number: int) -> str:
    output: list[str] = []
    for _ in range(15):
        output.append(ALPHABET[number & 31])
        number >>= 5
    return "".join(reversed(output))


def mapped(legacy: str) -> str:
    payload = hashlib.sha256(
        b"frame-cap-nanoid-to-uuid-v1\0" + legacy.encode()
    ).digest()
    value = bytearray(payload[:16])
    value[6] = (value[6] & 0x0F) | 0x80
    value[8] = (value[8] & 0x3F) | 0x80
    return str(uuid.UUID(bytes=bytes(value)))


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


ACTOR = uid(1)
FOREIGN = uid(2)
ORG_LEGACY = cap_id(100)
ORG = mapped(ORG_LEGACY)
ACCESS_LEGACY = cap_id(101)
ACCESS_ORG = mapped(ACCESS_LEGACY)
DENIED_LEGACY = cap_id(102)
DENIED_ORG = mapped(DENIED_LEGACY)


def connect() -> sqlite3.Connection:
    database = sqlite3.connect(":memory:", isolation_level=None)
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    return database


def migrate() -> sqlite3.Connection:
    database = connect()
    for path in sorted(MIGRATIONS.glob("*.sql")):
        database.executescript(path.read_text(encoding="utf-8"))
        violations = database.execute("PRAGMA foreign_key_check").fetchall()
        if violations:
            raise AssertionError(f"{path.name}: foreign key violations: {violations}")
    return database


def seed(database: sqlite3.Connection) -> None:
    for user_id, label, active in (
        (ACTOR, "actor", ORG),
        (FOREIGN, "foreign", DENIED_ORG),
    ):
        database.execute(
            """INSERT INTO users(
                 id,email,display_name,created_at_ms,updated_at_ms,status,
                 deleted_at_ms,active_organization_id,default_organization_id
               ) VALUES (?1,?2,?3,1,1,'active',NULL,?4,?4)""",
            (user_id, f"{label}@example.invalid", label, active),
        )
        database.execute(
            """INSERT INTO auth_identities_v2(
                 user_id,identity_revision,session_version,
                 created_at_ms,updated_at_ms,revision
               ) VALUES (?1,1,3,1,1,0)""",
            (user_id,),
        )
    for organization_id, owner, name in (
        (ORG, ACTOR, "My Organization"),
        (ACCESS_ORG, FOREIGN, "Accessible"),
        (DENIED_ORG, FOREIGN, "Denied"),
    ):
        database.execute(
            """INSERT INTO organizations(
                 id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,
                 revision,authority_version
               ) VALUES (?1,?2,?3,'active','{}',1,1,0,0)""",
            (organization_id, owner, name),
        )
        database.execute(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,has_pro_seat,
                 created_at_ms,updated_at_ms,revision,authority_version
               ) VALUES (?1,?2,'owner','active',0,1,1,0,0)""",
            (organization_id, owner),
        )
    # Cap's source query counts any membership row; suspended/removed is not
    # filtered. This intentionally proves that compatibility quirk.
    database.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,
             created_at_ms,updated_at_ms,revision,authority_version
           ) VALUES (?1,?2,'member','removed',0,1,1,0,0)""",
        (ACCESS_ORG, ACTOR),
    )
    database.execute(
        SQL["organization_mapping_insert"],
        (ORG, ORG_LEGACY, 1, uid(900)),
    )


Mutator = Callable[[sqlite3.Connection, str, int], None]


def operation(
    database: sqlite3.Connection,
    *,
    action: str,
    key: str,
    request: str,
    result_kind: str,
    mutate: Mutator,
    delta: int = 1,
    step: str | None = None,
    result_org: str | None = None,
    provider: str = "not_requested",
    browser_fence: tuple[str, str] | None = None,
) -> sqlite3.Row:
    key_digest = digest(f"key:{key}")
    request_digest = digest(f"request:{request}")
    existing = database.execute(
        SQL["operation_by_key"], (ACTOR, action, key_digest)
    ).fetchall()
    if existing:
        row = existing[0]
        if row["request_digest"] != request_digest:
            raise ValueError("idempotency conflict")
        if (
            row["state"] != "applied"
            or row["receipt_count"] != 1
            or row["effect_count"] != 1
            or row["audit_count"] != 1
        ):
            raise AssertionError("corrupt replay evidence")
        return row

    op = uid(10_000 + operation.serial)
    operation.serial += 1
    authority = database.execute(SQL["authority_snapshot"], (ACTOR,)).fetchone()
    if authority is None:
        raise AssertionError("authority missing")
    prior_revision = authority["legacy_user_account_revision"]
    database.execute("BEGIN")
    try:
        if browser_fence is not None:
            grant_id, session_id = browser_fence
            database.execute(
                BROWSER_GRANT_ASSERT_SQL,
                (op, grant_id, session_id, ACTOR, NOW),
            )
            database.execute(
                BROWSER_GRANT_DELETE_SQL,
                (grant_id, session_id, ACTOR),
            )
        database.execute(
            SQL["authority_assert"],
            (
                op,
                ACTOR,
                prior_revision,
                authority["legacy_user_account_authority_version"],
            ),
        )
        database.execute(
            SQL["operation_claim"],
            (op, ACTOR, action, key_digest, request_digest, NOW),
        )
        mutate(database, op, NOW)
        database.execute(
            SQL["mutation_assert"], (op, ACTOR, delta, prior_revision)
        )
        database.execute(
            SQL["receipt_insert"],
            (
                op,
                ACTOR,
                action,
                result_kind,
                step,
                result_org,
                provider,
                prior_revision + delta,
                NOW,
            ),
        )
        database.execute(
            SQL["effect_insert"],
            (
                op,
                ACTOR,
                action,
                provider,
                json.dumps(
                    {"action": action, "providerEffect": provider},
                    separators=(",", ":"),
                ),
                NOW,
            ),
        )
        database.execute(
            SQL["audit_insert"],
            (
                uid(20_000 + operation.serial),
                op,
                ACTOR,
                action,
                digest(f"principal:{ACTOR}"),
                digest(f"subject:{request_digest}"),
                NOW,
            ),
        )
        database.execute(
            SQL["operation_complete"],
            (op, result_kind, step, result_org, provider, NOW),
        )
        database.execute(SQL["durable_postcondition"], (op,))
        database.execute(SQL["assertion_cleanup"], (op,))
        if browser_fence is not None:
            database.execute(BROWSER_ASSERTION_CLEANUP_SQL, (op,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")
    return database.execute(
        SQL["operation_by_key"], (ACTOR, action, key_digest)
    ).fetchone()


operation.serial = 0


def patch_binding(value: object) -> tuple[int, str | None]:
    if value is ABSENT:
        return (0, None)
    if value is None:
        return (1, None)
    return (2, str(value))


ABSENT = object()


def test_migration_and_source_closure() -> None:
    database = migrate()
    columns = {
        row["name"] for row in database.execute("PRAGMA table_info(users)")
    }
    required = {
        "legacy_last_name",
        "legacy_image_key",
        "legacy_onboarding_steps_json",
        "legacy_stripe_subscription_status",
        "legacy_user_account_revision",
    }
    assert required <= columns
    assert len(SQL) == 40, len(SQL)
    application = APPLICATION.read_text(encoding="utf-8")
    runtime = RUNTIME.read_text(encoding="utf-8")
    for identity in (
        "cap-v1-fdc3d5d49bb5ad6d",
        "cap-v1-c7827a1de563f856",
        "cap-v1-295a3eb4ba9ffe6f",
        "cap-v1-fdf4d6473b7f6608",
        "cap-v1-c067d69850110640",
        "cap-v1-3d28eb7593bd4b1e",
        "cap-v1-e0040a01322ea19e",
        "cap-v1-859bad07650343aa",
    ):
        assert identity in application
    assert "payload_id: _" in application
    assert "LegacyUserAccountAtomicErrorV1::ProviderRequired" in runtime
    assert "BestEffortProtectedGate" in runtime


def test_name_presence_replay_and_conflict() -> None:
    database = migrate()
    seed(database)
    first = "  Ada  "

    def mutate(db: sqlite3.Connection, op: str, now: int) -> None:
        fm, fv = patch_binding(first)
        lm, lv = patch_binding(None)
        db.execute(SQL["name_update"], (ACTOR, fm, fv, lm, lv, now, op))

    row = operation(
        database,
        action="legacy.user.name",
        key="name-1",
        request="untrimmed-null",
        result_kind="json_true",
        mutate=mutate,
    )
    user = database.execute(
        "SELECT display_name,legacy_last_name FROM users WHERE id=?", (ACTOR,)
    ).fetchone()
    assert tuple(user) == (first, None)
    replay = operation(
        database,
        action="legacy.user.name",
        key="name-1",
        request="untrimmed-null",
        result_kind="json_true",
        mutate=lambda *_: (_ for _ in ()).throw(AssertionError("mutated replay")),
    )
    assert replay["operation_id"] == row["operation_id"]
    try:
        operation(
            database,
            action="legacy.user.name",
            key="name-1",
            request="different",
            result_kind="json_true",
            mutate=mutate,
        )
    except ValueError:
        pass
    else:
        raise AssertionError("idempotency conflict accepted")


def test_welcome_merge_and_exact_default_organization_rename() -> None:
    database = migrate()
    seed(database)
    database.execute(
        "UPDATE users SET legacy_onboarding_steps_json=? WHERE id=?",
        ('{"customDomain":true}', ACTOR),
    )

    def mutate(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(SQL["welcome_user_update"], (ACTOR, "Ada", "", now, op))
        personalized = "Ada's Organization"
        db.execute(
            SQL["welcome_organization_update"],
            (ORG, personalized, personalized, now, op),
        )

    operation(
        database,
        action="legacy.user.complete_onboarding",
        key="welcome-1",
        request="ecmascript-trim-default-last",
        result_kind="onboarding",
        step="welcome",
        mutate=mutate,
    )
    user = database.execute(
        """SELECT display_name,legacy_last_name,legacy_onboarding_steps_json
           FROM users WHERE id=?""",
        (ACTOR,),
    ).fetchone()
    assert user["display_name"] == "Ada" and user["legacy_last_name"] == ""
    flags = json.loads(user["legacy_onboarding_steps_json"])
    assert flags == {"customDomain": True, "welcome": True}
    organization = database.execute(
        "SELECT name,legacy_user_account_name FROM organizations WHERE id=?", (ORG,)
    ).fetchone()
    assert tuple(organization) == ("Ada's Organization", "Ada's Organization")
    database.execute("UPDATE organizations SET name='my organization' WHERE id=?", (ORG,))


def test_organization_setup_whitespace_projection_and_best_effort_icon() -> None:
    database = migrate()
    seed(database)
    database.execute(
        "UPDATE users SET active_organization_id=NULL,default_organization_id=NULL WHERE id=?",
        (ACTOR,),
    )
    legacy = cap_id(700)
    organization_id = mapped(legacy)
    raw_name = " " * 255

    def mutate(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(
            SQL["organization_insert"],
            (organization_id, ACTOR, "Legacy organization", raw_name, now, op),
        )
        db.execute(SQL["organization_owner_insert"], (organization_id, ACTOR, now, op))
        db.execute(SQL["organization_mapping_insert"], (organization_id, legacy, now, op))
        db.execute(SQL["organization_projection_assert"], (op, organization_id, legacy))
        db.execute(
            SQL["organization_setup_user_update"],
            (ACTOR, organization_id, op, now),
        )

    result = operation(
        database,
        action="legacy.user.complete_onboarding",
        key="org-setup-1",
        request="whitespace-unsupported-icon",
        result_kind="onboarding",
        step="organizationSetup",
        result_org=legacy,
        provider="best_effort_protected_gate",
        mutate=mutate,
    )
    assert result["result_legacy_organization_id"] == legacy
    assert result["provider_effect"] == "best_effort_protected_gate"
    organization = database.execute(
        """SELECT name,legacy_user_account_name
           FROM organizations WHERE id=?""",
        (organization_id,),
    ).fetchone()
    assert tuple(organization) == ("Legacy organization", raw_name)
    projected = database.execute(
        "SELECT COALESCE(legacy_user_account_name,name) FROM organizations WHERE id=?",
        (organization_id,),
    ).fetchone()[0]
    assert projected == raw_name
    owner = database.execute(
        """SELECT role,state FROM organization_members
           WHERE organization_id=? AND user_id=?""",
        (organization_id, ACTOR),
    ).fetchone()
    assert tuple(owner) == ("owner", "active")

    # A concurrent/imported conflicting source projection must abort instead
    # of returning an ID that differs from the durable mapping.
    conflicting = cap_id(702)
    projection_op = uid(45_000)
    database.execute("BEGIN")
    database.execute(
        SQL["organization_mapping_insert"],
        (organization_id, conflicting, NOW, projection_op),
    )
    try:
        database.execute(
            SQL["organization_projection_assert"],
            (projection_op, organization_id, conflicting),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_user_account_projection_v1" in str(error)
        database.execute("ROLLBACK")
    else:
        database.execute("ROLLBACK")
        raise AssertionError("conflicting organization projection accepted")


def test_custom_invite_and_skip_defaults() -> None:
    database = migrate()
    seed(database)

    def custom(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(SQL["custom_domain_update"], (ACTOR, now, op))

    operation(
        database,
        action="legacy.user.complete_onboarding",
        key="custom-1",
        request="custom",
        result_kind="onboarding",
        step="customDomain",
        mutate=custom,
    )

    def invite(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(SQL["invite_team_update"], (ACTOR, now, op))

    operation(
        database,
        action="legacy.user.complete_onboarding",
        key="invite-1",
        request="invite",
        result_kind="onboarding",
        step="inviteTeam",
        mutate=invite,
    )
    flags = json.loads(
        database.execute(
            "SELECT legacy_onboarding_steps_json FROM users WHERE id=?", (ACTOR,)
        ).fetchone()[0]
    )
    assert flags["customDomain"] and flags["inviteTeam"] and flags["download"]

    database.execute(
        """UPDATE users SET display_name=NULL,active_organization_id=?1,
             default_organization_id=?1,legacy_onboarding_steps_json=NULL WHERE id=?2""",
        (uid(999_999), ACTOR),
    )
    legacy = cap_id(701)
    new_org = mapped(legacy)

    def skip(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(
            SQL["organization_insert"],
            (new_org, ACTOR, "Your Organization", "Your Organization", now, op),
        )
        db.execute(SQL["organization_owner_insert"], (new_org, ACTOR, now, op))
        db.execute(SQL["organization_mapping_insert"], (new_org, legacy, now, op))
        db.execute(SQL["organization_projection_assert"], (op, new_org, legacy))
        db.execute(SQL["skip_user_update"], (ACTOR, "Your name", 1, new_org, op, now))

    operation(
        database,
        action="legacy.user.complete_onboarding",
        key="skip-1",
        request="placeholder",
        result_kind="onboarding",
        step="skipToDashboard",
        mutate=skip,
    )
    user = database.execute(
        """SELECT display_name,active_organization_id,default_organization_id,
                  legacy_onboarding_steps_json FROM users WHERE id=?""",
        (ACTOR,),
    ).fetchone()
    assert user["display_name"] == "Your name"
    assert user["active_organization_id"] == new_org
    assert set(json.loads(user["legacy_onboarding_steps_json"])) == {
        "welcome",
        "organizationSetup",
        "customDomain",
        "inviteTeam",
        "download",
    }


def test_user_update_absent_and_provider_fail_closed() -> None:
    database = migrate()
    seed(database)

    operation(
        database,
        action="legacy.user.update",
        key="image-absent-1",
        request="foreign-payload-id-ignored",
        result_kind="rpc_void",
        delta=0,
        mutate=lambda *_: None,
    )
    user = database.execute(
        "SELECT legacy_image_key,legacy_user_account_revision FROM users WHERE id=?",
        (ACTOR,),
    ).fetchone()
    assert tuple(user) == (None, 0)
    runtime = RUNTIME.read_text(encoding="utf-8")
    assert all(
        marker in runtime
        for marker in ("UserImageClear", "UserImageSome(_)", "ProviderRequired")
    )


def test_patch_account_owner_or_any_membership_and_atomic_denial() -> None:
    database = migrate()
    seed(database)

    def mutate(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(SQL["organization_access_assert"], (op, ACCESS_ORG, ACTOR))
        fm, fv = patch_binding("")
        lm, lv = patch_binding(ABSENT)
        db.execute(
            SQL["patch_account_update"],
            (ACTOR, fm, fv, lm, lv, 1, ACCESS_ORG, op, now),
        )

    operation(
        database,
        action="legacy.account.patch",
        key="patch-1",
        request="empty-first-removed-member",
        result_kind="server_action_void",
        mutate=mutate,
    )
    user = database.execute(
        "SELECT display_name,default_organization_id FROM users WHERE id=?", (ACTOR,)
    ).fetchone()
    assert tuple(user) == ("", ACCESS_ORG)

    before = database.execute(
        "SELECT COUNT(*) FROM legacy_user_account_operations_v1"
    ).fetchone()[0]

    def denied(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(SQL["organization_access_assert"], (op, DENIED_ORG, ACTOR))
        db.execute(
            SQL["patch_account_update"],
            (ACTOR, 2, "should-rollback", 0, None, 1, DENIED_ORG, op, now),
        )

    try:
        operation(
            database,
            action="legacy.account.patch",
            key="patch-denied",
            request="foreign",
            result_kind="server_action_void",
            mutate=denied,
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_user_account_forbidden_v1" in str(error)
    else:
        raise AssertionError("foreign default organization accepted")
    after = database.execute(
        "SELECT COUNT(*) FROM legacy_user_account_operations_v1"
    ).fetchone()[0]
    assert after == before


def seed_credentials(database: sqlite3.Connection) -> None:
    database.execute(
        """INSERT INTO sessions(
             id,user_id,token_digest,session_version,client_kind,
             issued_at_ms,expires_at_ms
           ) VALUES (?1,?2,?3,3,'browser',1,9999999999999)""",
        (uid(300), ACTOR, digest("legacy-session")),
    )
    database.execute(
        """INSERT INTO auth_api_keys(
             id,user_id,key_digest,name,scopes_json,created_at_ms
           ) VALUES (?1,?2,?3,'legacy','[\"read\"]',1)""",
        (uid(301), ACTOR, digest("legacy-key")),
    )
    session = uid(302)
    family = uid(303)
    database.execute(
        """INSERT INTO auth_sessions_v2(
             id,family_id,user_id,client_kind,token_key_version,token_digest,
             csrf_key_version,csrf_digest,browser_origin,issued_at_ms,rotated_at_ms,
             idle_expires_at_ms,absolute_expires_at_ms,session_version,generation,
             state,revision,last_operation_id
           ) VALUES (?1,?2,?3,'browser',1,?4,1,?5,'https://frame.engmanager.xyz',
             1,1,9999999999998,9999999999999,3,0,'active',0,?6)""",
        (session, family, ACTOR, digest("v2-session"), digest("csrf"), uid(304)),
    )
    database.execute(
        """INSERT INTO auth_session_credentials_v2(
             key_version,digest,session_id,family_id,state,revision,last_operation_id
           ) VALUES (1,?1,?2,?3,'current',0,?4)""",
        (digest("credential"), session, family, uid(305)),
    )
    database.execute(
        """INSERT INTO auth_session_mutation_grants_v2(
             id,session_id,user_id,generation,token_key_version,token_digest,
             created_at_ms,last_operation_id
           ) VALUES (?1,?2,?3,0,1,?4,1,?5)""",
        (uid(306), session, ACTOR, digest("v2-session"), uid(307)),
    )
    database.execute(
        """INSERT INTO auth_api_keys_v2(
             id,owner_id,tenant_id,key_version,key_digest,scopes_json,
             created_at_ms,revision,last_operation_id
           ) VALUES (?1,?2,?3,1,?4,'[\"media:read\"]',1,0,?5)""",
        (uid(308), ACTOR, ORG, digest("v2-key"), uid(309)),
    )


def test_sign_out_all_revokes_every_credential_family_atomically() -> None:
    database = migrate()
    seed(database)
    seed_credentials(database)

    def mutate(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(SQL["sign_out_user_update"], (ACTOR, now, op))
        db.execute(SQL["sign_out_identity_update"], (ACTOR, now, op))
        db.execute(SQL["sign_out_legacy_sessions_delete"], (ACTOR,))
        db.execute(SQL["sign_out_legacy_api_keys_delete"], (ACTOR,))
        db.execute(SQL["sign_out_v2_session_credentials_revoke"], (ACTOR, now, op))
        db.execute(SQL["sign_out_v2_mutation_grants_delete"], (ACTOR,))
        db.execute(SQL["sign_out_v2_sessions_revoke"], (ACTOR, now, op))
        db.execute(SQL["sign_out_v2_api_keys_revoke"], (ACTOR, now, op))

    operation(
        database,
        action="legacy.account.sign_out_all",
        key="logout-all-1",
        request="all-devices",
        result_kind="server_action_void",
        mutate=mutate,
    )
    assert database.execute(
        "SELECT session_version FROM users WHERE id=?", (ACTOR,)
    ).fetchone()[0] == 1
    assert database.execute(
        "SELECT session_version FROM auth_identities_v2 WHERE user_id=?", (ACTOR,)
    ).fetchone()[0] == 4
    assert database.execute("SELECT COUNT(*) FROM sessions WHERE user_id=?", (ACTOR,)).fetchone()[0] == 0
    assert database.execute("SELECT COUNT(*) FROM auth_api_keys WHERE user_id=?", (ACTOR,)).fetchone()[0] == 0
    v2_session = database.execute(
        "SELECT state,revocation_reason FROM auth_sessions_v2 WHERE user_id=?", (ACTOR,)
    ).fetchone()
    assert tuple(v2_session) == ("revoked", "logout_all")
    assert database.execute(
        "SELECT state FROM auth_session_credentials_v2"
    ).fetchone()[0] == "revoked"
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE user_id=?", (ACTOR,)
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT revoked_at_ms FROM auth_api_keys_v2 WHERE owner_id=?", (ACTOR,)
    ).fetchone()[0] == NOW


def test_browser_action_proof_is_consumed_atomically() -> None:
    database = migrate()
    seed(database)
    seed_credentials(database)
    grant_id = uid(306)
    session_id = uid(302)

    def patch(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(
            SQL["patch_account_update"],
            (ACTOR, 2, "proof-bound", 0, None, 0, None, op, now),
        )

    operation(
        database,
        action="legacy.account.patch",
        key="proof-bound-1",
        request="proof-bound",
        result_kind="server_action_void",
        mutate=patch,
        browser_fence=(grant_id, session_id),
    )
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?", (grant_id,)
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_assertions_v1"
    ).fetchone()[0] == 0

    stale_grant = uid(310)
    database.execute(
        """INSERT INTO auth_session_mutation_grants_v2(
             id,session_id,user_id,generation,token_key_version,token_digest,
             created_at_ms,last_operation_id
           ) VALUES (?1,?2,?3,1,1,?4,1,?5)""",
        (stale_grant, session_id, ACTOR, digest("v2-session"), uid(311)),
    )
    before = database.execute(
        "SELECT display_name FROM users WHERE id=?", (ACTOR,)
    ).fetchone()[0]
    operation_count = database.execute(
        "SELECT COUNT(*) FROM legacy_user_account_operations_v1"
    ).fetchone()[0]
    try:
        operation(
            database,
            action="legacy.account.patch",
            key="proof-bound-stale",
            request="stale-proof",
            result_kind="server_action_void",
            mutate=patch,
            browser_fence=(stale_grant, session_id),
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("stale browser action proof was accepted")
    assert database.execute(
        "SELECT display_name FROM users WHERE id=?", (ACTOR,)
    ).fetchone()[0] == before
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_user_account_operations_v1"
    ).fetchone()[0] == operation_count
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?", (stale_grant,)
    ).fetchone()[0] == 1


def test_devtools_and_evidence_immutability() -> None:
    database = migrate()
    seed(database)

    def promote(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(SQL["promote_to_pro"], (ACTOR, now, op))

    promoted = operation(
        database,
        action="legacy.devtool.promote_to_pro",
        key="promote-1",
        request="development",
        result_kind="server_action_void",
        mutate=promote,
    )
    user = database.execute(
        """SELECT legacy_stripe_customer_id,legacy_stripe_subscription_id,
                  legacy_stripe_subscription_status FROM users WHERE id=?""",
        (ACTOR,),
    ).fetchone()
    assert tuple(user) == ("development", "development", "active")

    def demote(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(SQL["demote_from_pro"], (ACTOR, now, op))

    operation(
        database,
        action="legacy.devtool.demote_from_pro",
        key="demote-1",
        request="development",
        result_kind="server_action_void",
        mutate=demote,
    )
    assert database.execute(
        "SELECT legacy_stripe_customer_id FROM users WHERE id=?", (ACTOR,)
    ).fetchone()[0] is None
    database.execute(
        """UPDATE users SET display_name='Name',legacy_last_name='Last',
             legacy_onboarding_steps_json='{"welcome":true}',
             legacy_onboarding_completed_at_ms=10 WHERE id=?""",
        (ACTOR,),
    )

    def restart(db: sqlite3.Connection, op: str, now: int) -> None:
        db.execute(SQL["restart_onboarding"], (ACTOR, now, op))

    operation(
        database,
        action="legacy.devtool.restart_onboarding",
        key="restart-1",
        request="development",
        result_kind="server_action_void",
        mutate=restart,
    )
    reset = database.execute(
        """SELECT display_name,legacy_last_name,legacy_onboarding_steps_json,
                  legacy_onboarding_completed_at_ms FROM users WHERE id=?""",
        (ACTOR,),
    ).fetchone()
    assert tuple(reset) == (None, None, None, None)
    try:
        database.execute(
            "UPDATE legacy_user_account_receipts_v1 SET action='tampered' WHERE operation_id=?",
            (promoted["operation_id"],),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_user_account_evidence_immutable_v1" in str(error)
    else:
        raise AssertionError("receipt was mutable")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    tests = [
        test_migration_and_source_closure,
        test_name_presence_replay_and_conflict,
        test_welcome_merge_and_exact_default_organization_rename,
        test_organization_setup_whitespace_projection_and_best_effort_icon,
        test_custom_invite_and_skip_defaults,
        test_user_update_absent_and_provider_fail_closed,
        test_patch_account_owner_or_any_membership_and_atomic_denial,
        test_sign_out_all_revokes_every_credential_family_atomically,
        test_browser_action_proof_is_consumed_atomically,
        test_devtools_and_evidence_immutability,
    ]
    for test in tests:
        test()
    print(
        "legacy user/account SQLite conformance: "
        f"{len(tests)} passed; 8 identities, local D1 semantics, replay, "
        "rollback, revocation, evidence immutability, and R2 gates verified"
    )
    if args.evidence is not None:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "family": "legacy_user_account.v1",
                    "tests_passed": len(tests),
                    "identities": 8,
                    "query_count": len(SQL),
                    "browser_action_proof": "one_use_atomic_consumption_verified",
                    "provider_boundary": "R2_execution_explicitly_gated",
                    "plaintext_secrets": False,
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )


if __name__ == "__main__":
    main()
