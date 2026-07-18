#!/usr/bin/env python3
"""SQLite/R2-state proof for the 21 provider-free organization/library actions."""

from __future__ import annotations

import base64
import hashlib
import json
import re
import sqlite3
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_organization_library"
FIXTURE = ROOT / "fixtures/api-parity/v1/organization-library.json"
APPLICATION = ROOT / "crates/application/src/legacy_organization_library.rs"
RUNTIME = ROOT / "apps/control-plane/src/legacy_organization_library_runtime.rs"
WEB_RUNTIME = ROOT / "apps/control-plane/src/legacy_organization_library_web_runtime.rs"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}

NOW = 1_735_787_045_006
OWNER = "00000000-0000-7000-8000-000000000001"
ADMIN = "00000000-0000-7000-8000-000000000002"
MEMBER = "00000000-0000-7000-8000-000000000003"
ORG = "00000000-0000-7000-8000-000000000101"
SPACE = "00000000-0000-7000-8000-000000000201"
FOLDER = "00000000-0000-7000-8000-000000000202"
STORAGE_R2 = "00000000-0000-7000-8000-000000000301"
STORAGE_DRIVE = "00000000-0000-7000-8000-000000000302"
SESSION = "00000000-0000-7000-8000-000000000401"
FAMILY = "00000000-0000-7000-8000-000000000402"
MEMBER_ALIAS = "0123456789abcde"

EXPECTED_OPERATIONS = {
    "cap-v1-2cbdd906b6b7e371",
    "cap-v1-61e089033a34d239",
    "cap-v1-79eeb0016e42f711",
    "cap-v1-120aa129daa79b1e",
    "cap-v1-9227b0da852f2745",
    "cap-v1-575866e31832347a",
    "cap-v1-3a1228254de4338a",
    "cap-v1-1bed8d446a1553b1",
    "cap-v1-b5f1312195f03a0e",
    "cap-v1-7e1553af9e9427af",
    "cap-v1-ff1b0a4f37fb9130",
    "cap-v1-ce276ebd911b73f8",
    "cap-v1-531e69b5e2915e10",
    "cap-v1-408f009a56471811",
    "cap-v1-dd736ee15a42f26b",
    "cap-v1-0d56f082dce4f861",
    "cap-v1-989b3a5027a3f5c0",
    "cap-v1-91184d308c393034",
    "cap-v1-1ffe1392bb59f2ca",
    "cap-v1-67377a620262de2c",
    "cap-v1-404c6ea8306ad5a7",
}


def uid(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def password_hash(password: str) -> str:
    salt = b"frame-test-salt!"
    assert len(salt) == 16
    derived = hashlib.pbkdf2_hmac("sha256", password.encode(), salt, 100_000, 32)
    return base64.b64encode(salt + derived).decode()


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:", isolation_level=None)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA foreign_keys = ON")
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
        assert not connection.execute("PRAGMA foreign_key_check").fetchall(), migration.name
    return connection


def compile_all_queries(connection: sqlite3.Connection) -> None:
    assert len(SQL) >= 40
    for name, statement in SQL.items():
        placeholders = [int(value) for value in re.findall(r"\?(\d+)", statement)]
        binding_count = max(placeholders, default=0)
        try:
            connection.execute("EXPLAIN " + statement, [None] * binding_count).fetchall()
        except sqlite3.Error as error:
            raise AssertionError(f"query {name}.sql does not compile: {error}") from error


def seed(connection: sqlite3.Connection) -> None:
    connection.executemany(
        """INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms)
           VALUES(?,?,?,?,?)""",
        [
            (OWNER, "owner@example.test", "Owner", 1, 1),
            (ADMIN, "admin@example.test", "Admin", 1, 1),
            (MEMBER, "member@example.test", "Member", 1, 1),
        ],
    )
    connection.execute(
        """UPDATE users SET legacy_stripe_subscription_id='sub_owner',
             legacy_stripe_subscription_status='active',legacy_invite_quota=3
           WHERE id=?""",
        (OWNER,),
    )
    connection.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,
             legacy_icon_key,legacy_shareable_link_icon_key,
             legacy_workos_organization_id,legacy_workos_connection_id
           ) VALUES(?,?,?,'active','{}',1,1,?,?,?,?)""",
        (
            ORG,
            OWNER,
            "Frame",
            f"organizations/{ORG}/icon/original.png",
            f"organizations/{ORG}/shareable-links/original.png",
            "org_workos",
            "conn_workos",
        ),
    )
    connection.executemany(
        "UPDATE users SET active_organization_id=? WHERE id=?",
        [(ORG, OWNER), (ORG, ADMIN), (ORG, MEMBER)],
    )
    connection.executemany(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,?,'active',?,1,1)""",
        [(ORG, OWNER, "owner", 1), (ORG, ADMIN, "admin", 0), (ORG, MEMBER, "member", 0)],
    )
    connection.execute(
        """INSERT INTO spaces(
             id,organization_id,created_by_user_id,name,is_public,settings_json,
             created_at_ms,updated_at_ms,legacy_password_hash,legacy_password_revision,
             legacy_icon_key
           ) VALUES(?,?,?,'Library',1,'{}',1,1,?,1,?)""",
        (SPACE, ORG, OWNER, password_hash("correct horse"), f"organizations/{ORG}/spaces/{SPACE}/old.png"),
    )
    connection.executemany(
        """INSERT INTO space_members(space_id,user_id,role,created_at_ms,updated_at_ms)
           VALUES(?,?,?,1,1)""",
        [(SPACE, ADMIN, "manager"), (SPACE, MEMBER, "viewer")],
    )
    connection.execute(
        """INSERT INTO folders(
             id,organization_id,space_id,created_by_user_id,name,is_public,settings_json,
             created_at_ms,updated_at_ms
           ) VALUES(?,?,?,?,?,1,'{}',1,1)""",
        (FOLDER, ORG, SPACE, OWNER, "Folder"),
    )
    connection.execute(
        """INSERT INTO legacy_invite_lifecycle_member_aliases_v1(
             mapped_member_id,legacy_member_id,organization_id,user_id,created_at_ms
           ) VALUES(?,?,?,?,1)""",
        (uid(501), MEMBER_ALIAS, ORG, MEMBER),
    )
    connection.executemany(
        """INSERT INTO storage_integrations(
             id,organization_id,owner_user_id,provider,state,capabilities_json,
             credential_ciphertext,created_at_ms,updated_at_ms,
             capabilities_schema_version,capabilities_checksum
           ) VALUES(?,?,?,?,?,?,?,1,1,1,?)""",
        [
            (STORAGE_R2, ORG, OWNER, "r2", "active", '{"schema_version":1}', "sealed-r2", digest("r2-capabilities")),
            (
                STORAGE_DRIVE,
                ORG,
                OWNER,
                "google_drive",
                "disabled",
                '{"schema_version":1,"folderId":"root","email":"owner@example.test"}',
                "sealed-drive",
                digest("drive-capabilities"),
            ),
        ],
    )
    connection.execute(
        """INSERT INTO auth_identities_v2(
             user_id,identity_revision,session_version,created_at_ms,updated_at_ms
           ) VALUES(?,1,0,1,1)""",
        (OWNER,),
    )
    connection.execute(
        """INSERT INTO auth_sessions_v2(
             id,family_id,user_id,client_kind,token_key_version,token_digest,
             csrf_key_version,csrf_digest,browser_origin,issued_at_ms,rotated_at_ms,
             idle_expires_at_ms,absolute_expires_at_ms,session_version,generation,state,
             last_operation_id
           ) VALUES(?,?,?,'browser',1,?,1,?,'https://frame.engmanager.xyz',1,1,?,?,0,0,
                    'active',?)""",
        (SESSION, FAMILY, OWNER, digest("token"), digest("csrf"), NOW + 60_000, NOW + 120_000, uid(502)),
    )


def issue_grant(connection: sqlite3.Connection, number: int) -> str:
    grant = uid(600 + number)
    connection.execute(
        """INSERT INTO auth_session_mutation_grants_v2(
             id,session_id,user_id,generation,token_key_version,token_digest,
             created_at_ms,last_operation_id
           ) VALUES(?,?,?,0,1,?,?,?)""",
        (grant, SESSION, OWNER, digest("token"), NOW, uid(700 + number)),
    )
    return grant


def claim(
    connection: sqlite3.Connection,
    operation: str,
    action: str,
    number: int,
    organization: str | None = ORG,
) -> tuple[str, str]:
    key = digest(f"key:{number}")
    request = digest(f"request:{number}")
    connection.execute(
        SQL["operation_claim"],
        (operation, organization, OWNER, action, key, request, NOW),
    )
    return key, request


def assert_browser(
    connection: sqlite3.Connection, operation: str, grant: str
) -> None:
    connection.execute(
        SQL["browser_grant_assert"], (operation, grant, SESSION, OWNER, NOW)
    )


def consume_browser(
    connection: sqlite3.Connection, operation: str, grant: str
) -> None:
    connection.execute(SQL["browser_grant_delete"], (grant, SESSION, OWNER))
    connection.execute(SQL["changes_assert"], (operation, "browser_grant_consumed", 1))


def complete(
    connection: sqlite3.Connection,
    operation: str,
    action: str,
    request_digest: str,
    organization: str = ORG,
) -> None:
    result = '{"kind":"success"}'
    effects = '{"invalidation_paths":[],"set_verified_password_cookie":false,"r2_keys_written":[],"r2_keys_deleted":[]}'
    connection.execute(
        SQL["operation_complete"], (operation, organization, result, effects, NOW + 1)
    )
    connection.execute(
        SQL["audit_insert"],
        (uid(int(operation[-3:], 16) + 2000), operation, organization, digest(OWNER), action, request_digest, NOW),
    )


def organization_snapshot(connection: sqlite3.Connection) -> sqlite3.Row:
    rows = connection.execute(SQL["active_organization_snapshot"], (OWNER, ORG)).fetchall()
    assert len(rows) == 1
    return rows[0]


def assert_organization(
    connection: sqlite3.Connection,
    operation: str,
    row: sqlite3.Row,
    mode: str = "manager",
) -> None:
    connection.execute(
        SQL["organization_authority_assert"],
        (
            operation,
            OWNER,
            ORG,
            row["organization_preference_revision"],
            row["organization_revision"],
            row["organization_authority_version"],
            row["legacy_organization_library_revision"],
            mode,
        ),
    )


def prove_password_non_journaled(connection: sqlite3.Connection) -> None:
    before = connection.execute(
        "SELECT COUNT(*) FROM legacy_organization_library_operations_v1"
    ).fetchone()[0]
    row = connection.execute(SQL["password_snapshot"], (SPACE,)).fetchone()
    assert row is not None and row["collection_kind"] == "space"
    decoded = base64.b64decode(row["password_hash"])
    assert hashlib.pbkdf2_hmac("sha256", b"correct horse", decoded[:16], 100_000, 32) == decoded[16:]
    folder = connection.execute(SQL["password_snapshot"], (FOLDER,)).fetchone()
    assert folder is not None and folder["password_hash"] == row["password_hash"]
    after = connection.execute(
        "SELECT COUNT(*) FROM legacy_organization_library_operations_v1"
    ).fetchone()[0]
    assert before == after, "public password verification was journaled"


def prove_space_visibility_atomicity(connection: sqlite3.Connection) -> None:
    operation = uid(801)
    grant = issue_grant(connection, 1)
    row = connection.execute(SQL["space_snapshot"], (OWNER, ORG, SPACE)).fetchone()
    connection.execute("BEGIN")
    _, request = claim(connection, operation, "set_space_collection_visibility", 1)
    assert_browser(connection, operation, grant)
    connection.execute(
        SQL["space_authority_assert"],
        (
            operation,
            OWNER,
            ORG,
            SPACE,
            row["organization_revision"],
            row["organization_authority_version"],
            row["space_revision"],
            row["space_authority_version"],
            row["legacy_organization_library_revision"],
        ),
    )
    connection.execute(
        SQL["set_space_visibility"],
        (SPACE, 0, '{"title":"Private library"}', operation, NOW),
    )
    connection.execute(SQL["changes_assert"], (operation, "space_visibility_updated", 1))
    complete(connection, operation, "set_space_collection_visibility", request)
    consume_browser(connection, operation, grant)
    connection.execute(SQL["assertion_cleanup"], (operation,))
    connection.execute("COMMIT")
    stored = connection.execute(
        "SELECT is_public,json_extract(settings_json,'$.publicPage.title') AS title FROM spaces WHERE id=?",
        (SPACE,),
    ).fetchone()
    assert tuple(stored) == (0, "Private library")
    assert connection.execute(
        "SELECT state FROM legacy_organization_library_operations_v1 WHERE operation_id=?", (operation,)
    ).fetchone()[0] == "complete"


def prove_stale_authority_rolls_back(connection: sqlite3.Connection) -> None:
    operation = uid(802)
    grant = issue_grant(connection, 2)
    row = organization_snapshot(connection)
    connection.execute("BEGIN")
    try:
        claim(connection, operation, "update_organization_details", 2)
        assert_browser(connection, operation, grant)
        connection.execute(
            SQL["organization_authority_assert"],
            (
                operation,
                OWNER,
                ORG,
                row["organization_preference_revision"],
                row["organization_revision"] + 1,
                row["organization_authority_version"],
                row["legacy_organization_library_revision"],
                "manager",
            ),
        )
    except sqlite3.IntegrityError:
        connection.execute("ROLLBACK")
    else:
        raise AssertionError("stale organization authority was accepted")
    assert connection.execute(
        "SELECT 1 FROM legacy_organization_library_operations_v1 WHERE operation_id=?", (operation,)
    ).fetchone() is None
    assert connection.execute(
        "SELECT 1 FROM auth_session_mutation_grants_v2 WHERE id=?", (grant,)
    ).fetchone() is not None
    connection.execute("DELETE FROM auth_session_mutation_grants_v2 WHERE id=?", (grant,))


def prove_resumable_r2_delete(connection: sqlite3.Connection) -> None:
    operation = uid(803)
    grant = issue_grant(connection, 3)
    row = organization_snapshot(connection)
    old_key = row["legacy_shareable_link_icon_key"]
    result = '{"kind":"success"}'
    effects = json.dumps(
        {
            "invalidation_paths": ["/dashboard/settings/organization"],
            "set_verified_password_cookie": False,
            "r2_keys_written": [],
            "r2_keys_deleted": [old_key],
        },
        separators=(",", ":"),
    )
    connection.execute("BEGIN")
    _, request = claim(connection, operation, "remove_shareable_link_icon", 3)
    assert_browser(connection, operation, grant)
    assert_organization(connection, operation, row)
    connection.execute(
        SQL["patch_organization_branding"],
        (ORG, None, 1, None, operation, NOW),
    )
    connection.execute(SQL["changes_assert"], (operation, "organization_branding_updated", 1))
    connection.execute(
        SQL["r2_effect_insert"],
        (operation, 0, "delete", old_key, None, None, "pending", None),
    )
    connection.execute(
        SQL["operation_set_pending"], (operation, ORG, result, effects, NOW)
    )
    connection.execute(
        SQL["audit_insert"],
        (uid(2803), operation, ORG, digest(OWNER), "remove_shareable_link_icon", request, NOW),
    )
    consume_browser(connection, operation, grant)
    connection.execute(SQL["assertion_cleanup"], (operation,))
    connection.execute("COMMIT")
    connection.execute(SQL["operation_complete"], (operation, ORG, result, effects, NOW + 1))
    assert connection.execute(
        "SELECT state FROM legacy_organization_library_operations_v1 WHERE operation_id=?", (operation,)
    ).fetchone()[0] == "storage_pending"
    connection.execute(SQL["r2_effect_applied"], (operation, 0, NOW + 2))
    connection.execute(SQL["operation_complete"], (operation, ORG, result, effects, NOW + 2))
    assert connection.execute(
        "SELECT state FROM legacy_organization_library_operations_v1 WHERE operation_id=?", (operation,)
    ).fetchone()[0] == "complete"
    try:
        connection.execute(
            "UPDATE legacy_organization_library_operations_v1 SET result_json='{}' WHERE operation_id=?",
            (operation,),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_organization_library_operation_immutable_v1" in str(error)
    else:
        raise AssertionError("complete operation receipt was mutable")


def prove_storage_selection(connection: sqlite3.Connection) -> None:
    operation = uid(804)
    grant = issue_grant(connection, 4)
    row = organization_snapshot(connection)
    storage = connection.execute(SQL["storage_snapshot"], (ORG,)).fetchall()
    assert all(item["credential_ciphertext"] for item in storage)
    connection.execute("BEGIN")
    _, request = claim(connection, operation, "set_organization_storage_provider", 4)
    assert_browser(connection, operation, grant)
    assert_organization(connection, operation, row)
    connection.execute(SQL["disable_storage_integrations"], (ORG, NOW, operation))
    connection.execute(
        SQL["enable_storage_integration"],
        (ORG, '["google_drive"]', NOW, operation),
    )
    connection.execute(SQL["changes_assert"], (operation, "storage_provider_enabled", 1))
    complete(connection, operation, "set_organization_storage_provider", request)
    consume_browser(connection, operation, grant)
    connection.execute(SQL["assertion_cleanup"], (operation,))
    connection.execute("COMMIT")
    active = connection.execute(
        "SELECT provider FROM storage_integrations WHERE organization_id=? AND state='active'", (ORG,)
    ).fetchall()
    assert [row[0] for row in active] == ["google_drive"]


def prove_member_removal(connection: sqlite3.Connection) -> None:
    operation = uid(805)
    grant = issue_grant(connection, 5)
    row = connection.execute(SQL["member_snapshot"], (OWNER, ORG, MEMBER_ALIAS)).fetchone()
    assert row is not None and row["target_user_id"] == MEMBER
    connection.execute("BEGIN")
    _, request = claim(connection, operation, "remove_organization_member", 5)
    assert_browser(connection, operation, grant)
    connection.execute(
        SQL["member_authority_assert"],
        (
            operation,
            OWNER,
            ORG,
            MEMBER_ALIAS,
            row["target_revision"],
            row["target_authority_version"],
        ),
    )
    connection.execute(SQL["remove_member_space_memberships"], (MEMBER, ORG, NOW, operation))
    connection.execute(SQL["remove_member_invites"], (ORG, digest("member@example.test"), NOW, operation))
    connection.execute(SQL["remove_member_alias"], (MEMBER_ALIAS, NOW, operation))
    connection.execute(SQL["changes_assert"], (operation, "member_alias_removed", 1))
    connection.execute(SQL["remove_member"], (ORG, MEMBER, NOW, operation))
    connection.execute(SQL["changes_assert"], (operation, "member_removed", 1))
    complete(connection, operation, "remove_organization_member", request)
    consume_browser(connection, operation, grant)
    connection.execute(SQL["assertion_cleanup"], (operation,))
    connection.execute("COMMIT")
    assert connection.execute(
        "SELECT state FROM organization_members WHERE organization_id=? AND user_id=?", (ORG, MEMBER)
    ).fetchone()[0] == "removed"
    assert connection.execute(
        "SELECT removed_at_ms FROM legacy_invite_lifecycle_member_aliases_v1 WHERE legacy_member_id=?",
        (MEMBER_ALIAS,),
    ).fetchone()[0] == NOW


def prove_create_without_active_tenant(connection: sqlite3.Connection) -> None:
    operation = uid(806)
    grant = issue_grant(connection, 6)
    new_org = uid(910)
    legacy_org = "1123456789abcde"
    long_name = "N" * 161
    actor = connection.execute(SQL["actor_snapshot"], (OWNER,)).fetchone()
    connection.execute("BEGIN")
    _, request = claim(connection, operation, "create_organization", 6, None)
    assert_browser(connection, operation, grant)
    connection.execute(
        SQL["actor_assert"],
        (operation, OWNER, actor["organization_preference_revision"]),
    )
    connection.execute(SQL["organization_name_assert"], (operation, long_name))
    connection.execute(
        SQL["create_organization"], (new_org, OWNER, long_name, None, NOW, operation)
    )
    connection.execute(SQL["changes_assert"], (operation, "organization_created", 1))
    connection.execute(
        SQL["create_organization_member"], (new_org, OWNER, NOW, operation)
    )
    connection.execute(SQL["changes_assert"], (operation, "organization_owner_created", 1))
    connection.execute(
        SQL["create_organization_alias"], (new_org, legacy_org, NOW, operation)
    )
    connection.execute(SQL["changes_assert"], (operation, "organization_alias_created", 1))
    connection.execute(
        SQL["select_active_organization"], (OWNER, new_org, operation, NOW)
    )
    connection.execute(SQL["changes_assert"], (operation, "created_organization_selected", 1))
    complete(connection, operation, "create_organization", request, new_org)
    consume_browser(connection, operation, grant)
    connection.execute(SQL["assertion_cleanup"], (operation,))
    connection.execute("COMMIT")
    created = connection.execute(
        "SELECT name,legacy_user_account_name FROM organizations WHERE id=?", (new_org,)
    ).fetchone()
    assert len(created["name"]) == 160 and created["legacy_user_account_name"] == long_name
    stored_operation = connection.execute(
        "SELECT organization_id,state FROM legacy_organization_library_operations_v1 WHERE operation_id=?",
        (operation,),
    ).fetchone()
    assert tuple(stored_operation) == (new_org, "complete")


def prove_fixture_and_source_pins() -> None:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    operations = fixture["operations"]
    ids = {operation["id"] for operation in operations}
    assert len(operations) == len(ids) == 21 and ids == EXPECTED_OPERATIONS
    assert fixture["reference_commit"] == "6ba69561ac86b8efdb17616d6727f9638015546b"
    assert fixture["protected_gates"] == []
    application = APPLICATION.read_text(encoding="utf-8")
    runtime = RUNTIME.read_text(encoding="utf-8")
    web_runtime = WEB_RUNTIME.read_text(encoding="utf-8")
    for operation in operations:
        assert operation["id"] in application
        assert operation["legacy_identity"] in application
        source = ROOT / ".tmp/cap" / operation["source_path"]
        if source.exists():
            assert hashlib.sha256(source.read_bytes()).hexdigest() == operation["source_sha256"]
    assert "VerifyCollectionPassword" in runtime and "PASSWORD_SNAPSHOT_SQL" in runtime
    assert "authenticate_compatibility_mutation" in web_runtime
    assert "admit_edge_request" in web_runtime


def main() -> None:
    connection = database()
    compile_all_queries(connection)
    seed(connection)
    prove_password_non_journaled(connection)
    prove_space_visibility_atomicity(connection)
    prove_stale_authority_rolls_back(connection)
    prove_resumable_r2_delete(connection)
    prove_storage_selection(connection)
    prove_member_removal(connection)
    prove_create_without_active_tenant(connection)
    prove_fixture_and_source_pins()
    assert not connection.execute("PRAGMA foreign_key_check").fetchall()
    print(
        "legacy organization/library sqlite conformance: 8 proofs passed; "
        "21 source-pinned actions, public password non-journaling, atomic browser fences, "
        "stale rollback, resumable R2 deletes, storage selection, member removal, and creation"
    )


if __name__ == "__main__":
    main()
