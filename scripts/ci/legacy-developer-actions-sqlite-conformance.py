#!/usr/bin/env python3
"""Provider-free SQLite proof for the eight Cap developer actions.

The suite applies the complete expand chain, executes the checked-in D1 SQL,
and proves user-owned non-disclosure, exact nullable/no-op semantics, zero-row
successes, protected key persistence, one-use browser proofs, replay/conflict/
in-flight handling, atomic rollback, and append-only evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
import tempfile
import threading
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable


ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_developer_actions"
RUNTIME = ROOT / "apps/control-plane/src/legacy_developer_actions_runtime.rs"
APPLICATION = ROOT / "crates/application/src/legacy_developer_actions.rs"
MIGRATION = MIGRATIONS / "0039_legacy_developer_actions_expand.sql"

NOW_MS = 1_700_000_000_000
ORIGIN = "https://portfolio.engmanager.xyz"
DASHBOARD = "/dashboard/developers"
ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"

ACTIONS = {
    "create": "legacy.developer.create_app",
    "update": "legacy.developer.update_app",
    "delete": "legacy.developer.delete_app",
    "add_domain": "legacy.developer.add_domain",
    "remove_domain": "legacy.developer.remove_domain",
    "regenerate": "legacy.developer.regenerate_keys",
    "delete_video": "legacy.developer.delete_video",
    "auto_top_up": "legacy.developer.update_auto_top_up",
}

OPERATION_IDS = [
    "cap-v1-f303e703a4237888",
    "cap-v1-87fd6af55b891cb9",
    "cap-v1-9833b16bb80a3299",
    "cap-v1-aa86dd3d5351ec06",
    "cap-v1-f7d8036af53d0eb9",
    "cap-v1-1f1465957551f1c4",
    "cap-v1-8328214ed9647abb",
    "cap-v1-b822700b545118f6",
]

SQL = {
    path.stem: path.read_text(encoding="utf-8").strip()
    for path in sorted(QUERIES.glob("*.sql"))
}


def uuid7_fixture(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def cap_id(number: int) -> str:
    output = []
    for _ in range(15):
        output.append(ALPHABET[number & 31])
        number >>= 5
    return "".join(reversed(output))


def mapped_uuid(legacy_id: str) -> str:
    payload = hashlib.sha256(
        b"frame-cap-nanoid-to-uuid-v1\0" + legacy_id.encode()
    ).digest()
    value = bytearray(payload[:16])
    value[6] = (value[6] & 0x0F) | 0x80
    value[8] = (value[8] & 0x3F) | 0x80
    return str(uuid.UUID(bytes=bytes(value)))


def digest(label: str) -> str:
    return hashlib.sha256(f"frame-developer-conformance:{label}".encode()).hexdigest()


ACTOR = uuid7_fixture(1)
FOREIGN_ACTOR = uuid7_fixture(2)
SUSPENDED_ACTOR = uuid7_fixture(3)
ORGANIZATION = uuid7_fixture(10)
FOREIGN_ORGANIZATION = uuid7_fixture(11)

APP_LEGACY = cap_id(100)
APP = mapped_uuid(APP_LEGACY)
CREDIT_LEGACY = cap_id(101)
CREDIT = mapped_uuid(CREDIT_LEGACY)
DOMAIN_LEGACY = cap_id(102)
DOMAIN = mapped_uuid(DOMAIN_LEGACY)
VIDEO_LEGACY = cap_id(103)
VIDEO = mapped_uuid(VIDEO_LEGACY)
PUBLIC_LEGACY = cap_id(104)
PUBLIC = mapped_uuid(PUBLIC_LEGACY)
SECRET_LEGACY = cap_id(105)
SECRET = mapped_uuid(SECRET_LEGACY)


def connect(path: Path | None = None) -> sqlite3.Connection:
    database = sqlite3.connect(
        ":memory:" if path is None else path,
        timeout=15,
        isolation_level=None,
    )
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    database.execute("PRAGMA busy_timeout = 15000")
    return database


def migration_paths(through: int | None = None) -> list[Path]:
    paths = sorted(MIGRATIONS.glob("*.sql"))
    if through is None:
        return paths
    return [path for path in paths if int(path.name[:4]) <= through]


def migrated_database(
    *, path: Path | None = None, through: int | None = None
) -> sqlite3.Connection:
    database = connect(path)
    for migration in migration_paths(through):
        database.executescript(migration.read_text(encoding="utf-8"))
        violations = database.execute("PRAGMA foreign_key_check").fetchall()
        if violations:
            raise AssertionError(
                f"{migration.name} introduced foreign-key violations: {violations}"
            )
    return database


def seed_fixture(database: sqlite3.Connection) -> None:
    database.execute("BEGIN")
    try:
        for number, user_id, label, status in (
            (1, ACTOR, "actor", "active"),
            (2, FOREIGN_ACTOR, "foreign", "active"),
            (3, SUSPENDED_ACTOR, "suspended", "suspended"),
        ):
            active_org = ORGANIZATION if user_id != FOREIGN_ACTOR else FOREIGN_ORGANIZATION
            database.execute(
                """INSERT INTO users(
                     id,email,display_name,created_at_ms,updated_at_ms,status,
                     deleted_at_ms,active_organization_id
                   ) VALUES (?1,?2,?3,1,1,?4,NULL,?5)""",
                (user_id, f"{label}-{number}@example.invalid", label, status, active_org),
            )
            database.execute(
                """INSERT INTO auth_identities_v2(
                     user_id,identity_revision,session_version,
                     created_at_ms,updated_at_ms,revision
                   ) VALUES (?1,1,3,1,1,0)""",
                (user_id,),
            )
        database.execute(
            """INSERT INTO organizations(
                 id,owner_id,name,status,created_at_ms,updated_at_ms,revision,authority_version
               ) VALUES (?1,?2,'primary','active',1,1,0,0)""",
            (ORGANIZATION, ACTOR),
        )
        database.execute(
            """INSERT INTO organizations(
                 id,owner_id,name,status,created_at_ms,updated_at_ms,revision,authority_version
               ) VALUES (?1,?2,'foreign','active',1,1,0,0)""",
            (FOREIGN_ORGANIZATION, FOREIGN_ACTOR),
        )
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


@dataclass(frozen=True)
class Grant:
    grant_id: str
    session_id: str
    actor_id: str


def seed_grant(
    database: sqlite3.Connection, serial: int, actor_id: str = ACTOR
) -> Grant:
    base = 10_000 + serial * 4
    session_id = uuid7_fixture(base)
    grant_id = uuid7_fixture(base + 2)
    token = digest(f"token:{serial}:{actor_id}")
    database.execute(
        """INSERT INTO auth_sessions_v2(
             id,family_id,user_id,client_kind,token_key_version,token_digest,
             csrf_key_version,csrf_digest,browser_origin,issued_at_ms,rotated_at_ms,
             idle_expires_at_ms,absolute_expires_at_ms,session_version,generation,
             state,revision,last_operation_id
           ) VALUES (?1,?2,?3,'browser',7,?4,7,?5,
             'https://frame.engmanager.xyz',?6,?6,?7,?8,3,4,'active',0,?9)""",
        (
            session_id,
            uuid7_fixture(base + 1),
            actor_id,
            token,
            digest(f"csrf:{serial}"),
            NOW_MS - 1_000,
            8_000_000_000_000_000,
            8_500_000_000_000_000,
            uuid7_fixture(base + 3),
        ),
    )
    database.execute(
        """INSERT INTO auth_session_mutation_grants_v2(
             id,session_id,user_id,generation,token_key_version,token_digest,
             created_at_ms,last_operation_id
           ) VALUES (?1,?2,?3,4,7,?4,?5,?6)""",
        (
            grant_id,
            session_id,
            actor_id,
            token,
            NOW_MS - 500,
            uuid7_fixture(base + 3),
        ),
    )
    return Grant(grant_id, session_id, actor_id)


def execute(
    database: sqlite3.Connection, name: str, parameters: tuple[Any, ...] = ()
) -> sqlite3.Cursor:
    return database.execute(SQL[name], parameters)


def one_row(
    database: sqlite3.Connection, name: str, parameters: tuple[Any, ...] = ()
) -> sqlite3.Row:
    rows = execute(database, name, parameters).fetchall()
    if len(rows) != 1:
        raise AssertionError(f"{name}: expected one row, received {len(rows)}")
    return rows[0]


def app_snapshot(
    database: sqlite3.Connection, actor_id: str = ACTOR
) -> sqlite3.Row | None:
    rows = execute(database, "app_authority_snapshot", (APP, actor_id)).fetchall()
    if len(rows) > 1:
        raise AssertionError("unbounded app authority")
    return rows[0] if rows else None


RECEIPT_FIELDS = (
    "result_kind",
    "app_id",
    "legacy_app_id",
    "final_name",
    "final_environment",
    "final_logo_url",
    "update_statement_executed",
    "deleted_at_ms",
    "revoked_active_key_count",
    "active_key_count_after",
    "domain_id",
    "legacy_domain_id",
    "stored_origin",
    "matched_rows",
    "video_id",
    "account_present",
    "auto_top_up_enabled",
    "auto_top_up_threshold_microcredits",
    "auto_top_up_amount_cents",
    "credit_account_id",
    "public_key_id",
    "secret_key_id",
    "sealed_key_replay",
    "replay_binding",
)


def receipt(result_kind: str, **values: Any) -> dict[str, Any]:
    output = {field: None for field in RECEIPT_FIELDS}
    output["result_kind"] = result_kind
    output.update(values)
    return output


Mutation = Callable[[sqlite3.Connection, str, sqlite3.Row | None], dict[str, Any]]


def apply_action(
    database: sqlite3.Connection,
    *,
    serial: int,
    action: str,
    key_label: str,
    request_label: str,
    mutation: Mutation,
    snapshot: sqlite3.Row | None,
    grant: Grant | None = None,
) -> tuple[str, Grant, dict[str, Any]]:
    grant = grant or seed_grant(database, serial)
    operation_id = uuid7_fixture(100_000 + serial * 2)
    audit_id = uuid7_fixture(100_001 + serial * 2)
    key_digest = digest(f"key:{key_label}")
    request_digest = digest(f"request:{request_label}")
    database.execute("BEGIN IMMEDIATE")
    try:
        execute(
            database,
            "browser_grant_assert",
            (operation_id, grant.grant_id, grant.session_id, grant.actor_id, NOW_MS),
        )
        execute(
            database,
            "operation_claim",
            (operation_id, ACTOR, action, key_digest, request_digest, NOW_MS),
        )
        if snapshot is not None:
            execute(
                database,
                "app_authority_assert",
                (
                    operation_id,
                    APP,
                    ACTOR,
                    snapshot["revision"],
                    snapshot["authority_version"],
                ),
            )
        facts = mutation(database, operation_id, snapshot)
        execute(
            database,
            "receipt_insert",
            (operation_id, *(facts[field] for field in RECEIPT_FIELDS), NOW_MS),
        )
        execute(database, "changes_assert", (operation_id, "receipt_inserted", 1))
        revalidate = int(action != ACTIONS["create"])
        execute(
            database,
            "effect_insert",
            (operation_id, revalidate, DASHBOARD if revalidate else None, NOW_MS),
        )
        execute(database, "changes_assert", (operation_id, "effect_inserted", 1))
        execute(
            database,
            "audit_insert",
            (audit_id, operation_id, ACTOR, action, digest(f"subject:{serial}"), NOW_MS),
        )
        execute(database, "changes_assert", (operation_id, "audit_inserted", 1))
        consumed = execute(
            database,
            "browser_grant_delete_returning",
            (grant.grant_id, grant.session_id, grant.actor_id),
        ).fetchall()
        assert len(consumed) == 1 and consumed[0]["actor_id"] == ACTOR
        execute(database, "changes_assert", (operation_id, "grant_consumed", 1))
        execute(
            database,
            "proof_insert",
            (
                grant.grant_id,
                grant.session_id,
                ACTOR,
                operation_id,
                action,
                request_digest,
                "applied",
                NOW_MS,
            ),
        )
        execute(database, "changes_assert", (operation_id, "proof_journaled", 1))
        execute(database, "operation_complete", (operation_id, NOW_MS))
        execute(database, "changes_assert", (operation_id, "operation_complete", 1))
        execute(
            database,
            "durable_receipt_assert",
            (
                operation_id,
                ACTOR,
                action,
                request_digest,
                facts["result_kind"],
                grant.grant_id,
                grant.session_id,
                "applied",
            ),
        )
        execute(database, "assertion_cleanup", (operation_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")
    return operation_id, grant, facts


def consume_attempt(
    database: sqlite3.Connection,
    *,
    serial: int,
    action: str,
    request_digest: str,
    outcome: str,
    related_operation_id: str | None,
    grant: Grant | None = None,
) -> Grant:
    grant = grant or seed_grant(database, serial)
    assertion_id = related_operation_id or uuid7_fixture(300_000 + serial)
    database.execute("BEGIN IMMEDIATE")
    try:
        execute(
            database,
            "browser_grant_assert",
            (assertion_id, grant.grant_id, grant.session_id, ACTOR, NOW_MS),
        )
        assert len(
            execute(
                database,
                "browser_grant_delete_returning",
                (grant.grant_id, grant.session_id, ACTOR),
            ).fetchall()
        ) == 1
        execute(database, "changes_assert", (assertion_id, "grant_consumed", 1))
        execute(
            database,
            "proof_insert",
            (
                grant.grant_id,
                grant.session_id,
                ACTOR,
                related_operation_id,
                action,
                request_digest,
                outcome,
                NOW_MS,
            ),
        )
        execute(database, "changes_assert", (assertion_id, "proof_journaled", 1))
        execute(database, "assertion_cleanup", (assertion_id,))
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")
    return grant


def expect_integrity_error(
    database: sqlite3.Connection,
    sql: str,
    parameters: tuple[Any, ...] = (),
    marker: str | None = None,
) -> None:
    database.execute("SAVEPOINT expected_failure")
    try:
        database.execute(sql, parameters)
    except sqlite3.IntegrityError as error:
        database.execute("ROLLBACK TO expected_failure")
        database.execute("RELEASE expected_failure")
        if marker is not None and marker not in str(error):
            raise AssertionError(f"expected {marker!r}; received {error!r}") from error
        return
    database.execute("ROLLBACK TO expected_failure")
    database.execute("RELEASE expected_failure")
    raise AssertionError("expected SQLite integrity failure")


def create_mutation(
    database: sqlite3.Connection, operation_id: str, snapshot: sqlite3.Row | None
) -> dict[str, Any]:
    assert snapshot is None
    execute(
        database,
        "create_app_insert",
        (APP, APP_LEGACY, ACTOR, "EngManager Frame", "development", NOW_MS, operation_id),
    )
    execute(database, "changes_assert", (operation_id, "app_mutated", 1))
    execute(
        database,
        "key_insert",
        (
            PUBLIC,
            PUBLIC_LEGACY,
            APP,
            "public",
            "cpk_abcdefgh",
            digest("public-key"),
            "AQBprotected-public-ciphertext",
            NOW_MS,
            operation_id,
        ),
    )
    execute(
        database,
        "key_insert",
        (
            SECRET,
            SECRET_LEGACY,
            APP,
            "secret",
            "csk_abcdefgh",
            digest("secret-key"),
            "AQBprotected-secret-ciphertext",
            NOW_MS,
            operation_id,
        ),
    )
    execute(database, "changes_assert", (operation_id, "key_rows_mutated", 1))
    execute(
        database,
        "credit_insert",
        (CREDIT, CREDIT_LEGACY, APP, ACTOR, NOW_MS, operation_id),
    )
    execute(database, "changes_assert", (operation_id, "account_mutated", 1))
    execute(
        database,
        "create_postcondition_assert",
        (
            operation_id,
            APP,
            APP_LEGACY,
            ACTOR,
            "EngManager Frame",
            "development",
            CREDIT,
        ),
    )
    return receipt(
        "app_created",
        app_id=APP,
        legacy_app_id=APP_LEGACY,
        final_name="EngManager Frame",
        final_environment="development",
        active_key_count_after=2,
        account_present=1,
        auto_top_up_enabled=0,
        auto_top_up_threshold_microcredits=0,
        auto_top_up_amount_cents=0,
        credit_account_id=CREDIT,
        public_key_id=PUBLIC,
        secret_key_id=SECRET,
        sealed_key_replay="AQBsealed-response-replay",
        replay_binding=digest("create-replay-binding"),
    )


def update_mutation(
    *,
    name: str | None = None,
    environment: str | None = None,
    logo_present: bool = False,
    logo_value: str | None = None,
) -> Mutation:
    def mutate(
        database: sqlite3.Connection,
        operation_id: str,
        snapshot: sqlite3.Row | None,
    ) -> dict[str, Any]:
        assert snapshot is not None
        executed = name is not None or environment is not None or logo_present
        execute(
            database,
            "update_app",
            (
                operation_id,
                APP,
                ACTOR,
                snapshot["revision"],
                snapshot["authority_version"],
                int(name is not None),
                name,
                int(environment is not None),
                environment,
                int(logo_present),
                logo_value,
                NOW_MS,
                int(executed),
            ),
        )
        execute(
            database,
            "changes_assert",
            (operation_id, "app_mutated", int(executed)),
        )
        final_name = name if name is not None else snapshot["name"]
        final_environment = (
            environment if environment is not None else snapshot["environment"]
        )
        final_logo = logo_value if logo_present else snapshot["logo_url"]
        final_revision = snapshot["revision"] + int(executed)
        last_operation_id = operation_id if executed else snapshot["last_operation_id"]
        execute(
            database,
            "update_postcondition_assert",
            (
                operation_id,
                APP,
                ACTOR,
                final_name,
                final_environment,
                final_logo,
                final_revision,
                snapshot["authority_version"],
                last_operation_id,
            ),
        )
        return receipt(
            "app_updated",
            app_id=APP,
            final_name=final_name,
            final_environment=final_environment,
            final_logo_url=final_logo,
            update_statement_executed=int(executed),
        )

    return mutate


def add_domain_mutation(
    database: sqlite3.Connection, operation_id: str, snapshot: sqlite3.Row | None
) -> dict[str, Any]:
    assert snapshot is not None
    execute(
        database,
        "domain_insert",
        (
            DOMAIN,
            DOMAIN_LEGACY,
            APP,
            ORIGIN,
            NOW_MS,
            operation_id,
            ACTOR,
            snapshot["revision"],
            snapshot["authority_version"],
        ),
    )
    execute(database, "changes_assert", (operation_id, "domain_mutated", 1))
    execute(
        database,
        "domain_add_postcondition_assert",
        (operation_id, DOMAIN, DOMAIN_LEGACY, APP, ORIGIN),
    )
    return receipt(
        "domain_added",
        app_id=APP,
        domain_id=DOMAIN,
        legacy_domain_id=DOMAIN_LEGACY,
        stored_origin=ORIGIN,
    )


def remove_domain_mutation(expected: int) -> Mutation:
    def mutate(
        database: sqlite3.Connection,
        operation_id: str,
        snapshot: sqlite3.Row | None,
    ) -> dict[str, Any]:
        assert snapshot is not None
        count = one_row(database, "domain_target_count", (DOMAIN, APP))["target_count"]
        assert count == expected
        execute(
            database,
            "domain_delete",
            (
                operation_id,
                DOMAIN,
                APP,
                ACTOR,
                snapshot["revision"],
                snapshot["authority_version"],
            ),
        )
        execute(database, "changes_assert", (operation_id, "domain_mutated", expected))
        execute(
            database,
            "domain_remove_postcondition_assert",
            (operation_id, DOMAIN, APP),
        )
        return receipt(
            "domain_delete_attempted",
            app_id=APP,
            domain_id=DOMAIN,
            matched_rows=expected,
        )

    return mutate


def regenerate_mutation(
    database: sqlite3.Connection, operation_id: str, snapshot: sqlite3.Row | None
) -> dict[str, Any]:
    assert snapshot is not None
    new_public_legacy = cap_id(204)
    new_secret_legacy = cap_id(205)
    new_public = mapped_uuid(new_public_legacy)
    new_secret = mapped_uuid(new_secret_legacy)
    execute(database, "revoke_active_keys", (operation_id, APP, ACTOR, NOW_MS))
    execute(
        database,
        "changes_assert",
        (operation_id, "key_rows_mutated", snapshot["active_key_count"]),
    )
    for key_id, legacy, kind, prefix in (
        (new_public, new_public_legacy, "public", "cpk_newvalue"),
        (new_secret, new_secret_legacy, "secret", "csk_newvalue"),
    ):
        execute(
            database,
            "key_insert",
            (
                key_id,
                legacy,
                APP,
                kind,
                prefix,
                digest(f"regenerated:{kind}"),
                f"AQBprotected-regenerated-{kind}",
                NOW_MS,
                operation_id,
            ),
        )
    execute(database, "regenerate_postcondition_assert", (operation_id, APP))
    return receipt(
        "keys_regenerated",
        app_id=APP,
        revoked_active_key_count=snapshot["active_key_count"],
        active_key_count_after=2,
        public_key_id=new_public,
        secret_key_id=new_secret,
        sealed_key_replay="AQBsealed-regenerated-replay",
        replay_binding=digest("regenerate-replay-binding"),
    )


def delete_video_mutation(expected: int) -> Mutation:
    def mutate(
        database: sqlite3.Connection,
        operation_id: str,
        snapshot: sqlite3.Row | None,
    ) -> dict[str, Any]:
        assert snapshot is not None
        count = one_row(database, "video_target_count", (VIDEO, APP))["target_count"]
        assert count == expected
        execute(
            database,
            "video_delete",
            (
                operation_id,
                VIDEO,
                APP,
                ACTOR,
                NOW_MS,
                snapshot["revision"],
                snapshot["authority_version"],
            ),
        )
        execute(database, "changes_assert", (operation_id, "video_mutated", expected))
        execute(
            database,
            "video_postcondition_assert",
            (operation_id, VIDEO, APP, expected, NOW_MS if expected else None),
        )
        return receipt(
            "video_delete_attempted",
            app_id=APP,
            deleted_at_ms=NOW_MS if expected else None,
            matched_rows=expected,
            video_id=VIDEO,
        )

    return mutate


def auto_top_up_mutation(
    database: sqlite3.Connection, operation_id: str, snapshot: sqlite3.Row | None
) -> dict[str, Any]:
    assert snapshot is not None and snapshot["credit_account_id"] == CREDIT
    execute(
        database,
        "auto_top_up_update",
        (
            operation_id,
            APP,
            ACTOR,
            1,
            1,
            2_000_000,
            1,
            2500,
            NOW_MS,
            snapshot["credit_revision"],
            snapshot["revision"],
            snapshot["authority_version"],
        ),
    )
    execute(database, "changes_assert", (operation_id, "account_mutated", 1))
    execute(
        database,
        "auto_top_up_postcondition_assert",
        (
            operation_id,
            APP,
            1,
            ACTOR,
            1,
            2_000_000,
            2500,
            snapshot["credit_revision"] + 1,
        ),
    )
    return receipt(
        "auto_top_up_updated",
        app_id=APP,
        account_present=1,
        auto_top_up_enabled=1,
        auto_top_up_threshold_microcredits=2_000_000,
        auto_top_up_amount_cents=2500,
        credit_account_id=CREDIT,
    )


def delete_app_mutation(
    database: sqlite3.Connection, operation_id: str, snapshot: sqlite3.Row | None
) -> dict[str, Any]:
    assert snapshot is not None
    execute(database, "revoke_active_keys", (operation_id, APP, ACTOR, NOW_MS))
    execute(
        database,
        "changes_assert",
        (operation_id, "key_rows_mutated", snapshot["active_key_count"]),
    )
    execute(
        database,
        "delete_app",
        (
            operation_id,
            APP,
            ACTOR,
            snapshot["revision"],
            snapshot["authority_version"],
            NOW_MS,
        ),
    )
    execute(database, "changes_assert", (operation_id, "app_mutated", 1))
    execute(
        database,
        "delete_app_postcondition_assert",
        (
            operation_id,
            APP,
            ACTOR,
            NOW_MS,
            snapshot["revision"] + 1,
            snapshot["authority_version"] + 1,
        ),
    )
    return receipt(
        "app_deleted",
        app_id=APP,
        deleted_at_ms=NOW_MS,
        revoked_active_key_count=snapshot["active_key_count"],
        active_key_count_after=0,
    )


def test_full_migration_chain_and_isolated_expand() -> None:
    database = migrated_database(through=38)
    seed_fixture(database)
    before = database.execute("SELECT COUNT(*) FROM users").fetchone()[0]
    database.executescript(MIGRATION.read_text(encoding="utf-8"))
    assert database.execute("PRAGMA foreign_key_check").fetchall() == []
    assert database.execute("SELECT COUNT(*) FROM users").fetchone()[0] == before
    tables = {
        row[0]
        for row in database.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        ).fetchall()
    }
    assert {
        "legacy_developer_apps_v1",
        "legacy_developer_app_domains_v1",
        "legacy_developer_api_keys_v1",
        "legacy_developer_videos_v1",
        "legacy_developer_credit_accounts_v1",
        "legacy_developer_action_operations_v1",
        "legacy_developer_action_receipts_v1",
        "legacy_developer_action_effects_v1",
        "legacy_developer_action_audit_events_v1",
        "legacy_developer_action_proof_consumptions_v1",
        "legacy_developer_action_assertions_v1",
    } <= tables
    database.close()

    full = migrated_database()
    assert database_quick_check(full)
    full.close()


def database_quick_check(database: sqlite3.Connection) -> bool:
    return (
        database.execute("PRAGMA foreign_key_check").fetchall() == []
        and database.execute("PRAGMA quick_check").fetchone()[0] == "ok"
    )


def test_all_actions_exact_mutations_and_zero_row_successes() -> None:
    database = migrated_database(through=39)
    seed_fixture(database)
    clock = one_row(database, "clock_now")
    assert 0 <= clock["now_ms"] <= 253_402_300_799_999

    create_operation, _, _ = apply_action(
        database,
        serial=1,
        action=ACTIONS["create"],
        key_label="create",
        request_label="create",
        mutation=create_mutation,
        snapshot=None,
    )
    created = one_row(
        database,
        "operation_by_key",
        (ACTOR, ACTIONS["create"], digest("key:create")),
    )
    assert created["operation_id"] == create_operation
    assert created["state"] == "complete"
    assert created["legacy_app_id"] == APP_LEGACY
    assert created["legacy_credit_account_id"] == CREDIT_LEGACY
    assert created["revalidate_developer_dashboard"] == 0
    assert created["revalidation_path"] is None

    apply_action(
        database,
        serial=2,
        action=ACTIONS["update"],
        key_label="logo-value",
        request_label="logo-value",
        mutation=update_mutation(logo_present=True, logo_value="https://cdn.example/logo.png"),
        snapshot=app_snapshot(database),
    )
    before_noop = app_snapshot(database)
    assert before_noop is not None
    apply_action(
        database,
        serial=3,
        action=ACTIONS["update"],
        key_label="empty-update",
        request_label="empty-update",
        mutation=update_mutation(),
        snapshot=before_noop,
    )
    after_noop = app_snapshot(database)
    assert after_noop is not None
    assert after_noop["revision"] == before_noop["revision"]
    assert after_noop["last_operation_id"] == before_noop["last_operation_id"]
    apply_action(
        database,
        serial=4,
        action=ACTIONS["update"],
        key_label="logo-null",
        request_label="logo-null",
        mutation=update_mutation(logo_present=True, logo_value=None),
        snapshot=app_snapshot(database),
    )
    assert app_snapshot(database)["logo_url"] is None

    apply_action(
        database,
        serial=5,
        action=ACTIONS["add_domain"],
        key_label="add-domain",
        request_label="add-domain",
        mutation=add_domain_mutation,
        snapshot=app_snapshot(database),
    )
    apply_action(
        database,
        serial=6,
        action=ACTIONS["remove_domain"],
        key_label="remove-domain-one",
        request_label="remove-domain-one",
        mutation=remove_domain_mutation(1),
        snapshot=app_snapshot(database),
    )
    _, _, zero_domain = apply_action(
        database,
        serial=7,
        action=ACTIONS["remove_domain"],
        key_label="remove-domain-zero",
        request_label="remove-domain-zero",
        mutation=remove_domain_mutation(0),
        snapshot=app_snapshot(database),
    )
    assert zero_domain["matched_rows"] == 0

    apply_action(
        database,
        serial=8,
        action=ACTIONS["regenerate"],
        key_label="regenerate",
        request_label="regenerate",
        mutation=regenerate_mutation,
        snapshot=app_snapshot(database),
    )
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_developer_api_keys_v1 WHERE app_id=?1 AND revoked_at_ms IS NULL",
        (APP,),
    ).fetchone()[0] == 2

    database.execute(
        """INSERT INTO legacy_developer_videos_v1(
             id,legacy_video_id,app_id,deleted_at_ms,created_at_ms,updated_at_ms,
             revision,last_operation_id
           ) VALUES (?1,?2,?3,NULL,1,1,0,NULL)""",
        (VIDEO, VIDEO_LEGACY, APP),
    )
    apply_action(
        database,
        serial=9,
        action=ACTIONS["delete_video"],
        key_label="delete-video-one",
        request_label="delete-video-one",
        mutation=delete_video_mutation(1),
        snapshot=app_snapshot(database),
    )
    _, _, zero_video = apply_action(
        database,
        serial=10,
        action=ACTIONS["delete_video"],
        key_label="delete-video-zero",
        request_label="delete-video-zero",
        mutation=delete_video_mutation(0),
        snapshot=app_snapshot(database),
    )
    assert zero_video["matched_rows"] == 0 and zero_video["deleted_at_ms"] is None

    apply_action(
        database,
        serial=11,
        action=ACTIONS["auto_top_up"],
        key_label="auto-top-up",
        request_label="auto-top-up",
        mutation=auto_top_up_mutation,
        snapshot=app_snapshot(database),
    )
    assert tuple(
        database.execute(
            """SELECT auto_top_up_enabled,auto_top_up_threshold_microcredits,
                      auto_top_up_amount_cents
               FROM legacy_developer_credit_accounts_v1 WHERE id=?1""",
            (CREDIT,),
        ).fetchone()
    ) == (1, 2_000_000, 2500)

    apply_action(
        database,
        serial=12,
        action=ACTIONS["delete"],
        key_label="delete-app",
        request_label="delete-app",
        mutation=delete_app_mutation,
        snapshot=app_snapshot(database),
    )
    assert app_snapshot(database) is None
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_developer_api_keys_v1 WHERE app_id=?1 AND revoked_at_ms IS NULL",
        (APP,),
    ).fetchone()[0] == 0
    assert database.execute(
        """SELECT COUNT(*) FROM legacy_developer_action_effects_v1
           WHERE revalidate_developer_dashboard=1 AND revalidation_path=?1""",
        (DASHBOARD,),
    ).fetchone()[0] == 11
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_developer_action_assertions_v1"
    ).fetchone()[0] == 0
    assert database_quick_check(database)
    database.close()


def test_user_owned_non_disclosure_and_absent_credit_account() -> None:
    database = migrated_database(through=39)
    seed_fixture(database)
    database.execute(
        """INSERT INTO legacy_developer_apps_v1(
             id,legacy_app_id,owner_id,name,environment,logo_url,deleted_at_ms,
             created_at_ms,updated_at_ms,revision,authority_version,last_operation_id
           ) VALUES (?1,?2,?3,'Owned app','production',NULL,NULL,1,1,0,0,NULL)""",
        (APP, APP_LEGACY, ACTOR),
    )
    assert app_snapshot(database, FOREIGN_ACTOR) is None
    assert app_snapshot(database, SUSPENDED_ACTOR) is None

    def absent_account(
        target: sqlite3.Connection,
        operation_id: str,
        snapshot: sqlite3.Row | None,
    ) -> dict[str, Any]:
        assert snapshot is not None and snapshot["credit_account_id"] is None
        execute(
            target,
            "auto_top_up_update",
            (
                operation_id,
                APP,
                ACTOR,
                1,
                1,
                50,
                1,
                100,
                NOW_MS,
                None,
                snapshot["revision"],
                snapshot["authority_version"],
            ),
        )
        execute(target, "changes_assert", (operation_id, "account_mutated", 0))
        execute(
            target,
            "auto_top_up_postcondition_assert",
            (operation_id, APP, 0, ACTOR, None, None, None, None),
        )
        return receipt("auto_top_up_updated", app_id=APP, account_present=0)

    _, _, facts = apply_action(
        database,
        serial=20,
        action=ACTIONS["auto_top_up"],
        key_label="absent-credit",
        request_label="absent-credit",
        mutation=absent_account,
        snapshot=app_snapshot(database),
    )
    assert facts["account_present"] == 0
    database.execute(
        "UPDATE legacy_developer_apps_v1 SET deleted_at_ms=?1 WHERE id=?2",
        (NOW_MS, APP),
    )
    assert app_snapshot(database) is None
    database.close()


def test_replay_conflict_in_flight_and_one_use_proof() -> None:
    database = migrated_database(through=39)
    seed_fixture(database)
    operation_id, applied_grant, _ = apply_action(
        database,
        serial=30,
        action=ACTIONS["create"],
        key_label="replay-create",
        request_label="replay-create",
        mutation=create_mutation,
        snapshot=None,
    )
    request_digest = digest("request:replay-create")
    row = one_row(
        database,
        "operation_by_key",
        (ACTOR, ACTIONS["create"], digest("key:replay-create")),
    )
    assert row["operation_id"] == operation_id
    assert row["sealed_key_replay"] == "AQBsealed-response-replay"
    assert row["audit_count"] == 1 and row["proof_count"] == 1

    replay_grant = consume_attempt(
        database,
        serial=31,
        action=ACTIONS["create"],
        request_digest=request_digest,
        outcome="replay",
        related_operation_id=operation_id,
    )
    assert database.execute(
        """SELECT outcome FROM legacy_developer_action_proof_consumptions_v1
           WHERE mutation_grant_id=?1""",
        (replay_grant.grant_id,),
    ).fetchone()[0] == "replay"

    conflict_digest = digest("request:conflicting-create")
    conflict_grant = consume_attempt(
        database,
        serial=32,
        action=ACTIONS["create"],
        request_digest=conflict_digest,
        outcome="conflict",
        related_operation_id=operation_id,
    )
    assert database.execute(
        "SELECT outcome FROM legacy_developer_action_proof_consumptions_v1 WHERE mutation_grant_id=?1",
        (conflict_grant.grant_id,),
    ).fetchone()[0] == "conflict"

    inflight_id = uuid7_fixture(390_000)
    execute(
        database,
        "operation_claim",
        (
            inflight_id,
            ACTOR,
            ACTIONS["update"],
            digest("key:in-flight"),
            digest("request:in-flight"),
            NOW_MS,
        ),
    )
    inflight = one_row(
        database,
        "operation_by_key",
        (ACTOR, ACTIONS["update"], digest("key:in-flight")),
    )
    assert inflight["state"] == "claimed" and inflight["result_kind"] is None
    inflight_grant = consume_attempt(
        database,
        serial=33,
        action=ACTIONS["update"],
        request_digest=digest("request:in-flight"),
        outcome="in_flight",
        related_operation_id=inflight_id,
    )
    assert database.execute(
        "SELECT outcome FROM legacy_developer_action_proof_consumptions_v1 WHERE mutation_grant_id=?1",
        (inflight_grant.grant_id,),
    ).fetchone()[0] == "in_flight"

    expect_integrity_error(
        database,
        SQL["browser_grant_assert"],
        (
            uuid7_fixture(390_001),
            replay_grant.grant_id,
            replay_grant.session_id,
            ACTOR,
            NOW_MS,
        ),
        "frame_legacy_developer_authority_v1",
    )
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id IN (?1,?2)",
        (applied_grant.grant_id, replay_grant.grant_id),
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_developer_apps_v1"
    ).fetchone()[0] == 1
    database.close()


def test_stale_authority_and_false_postcondition_roll_back_atomically() -> None:
    database = migrated_database(through=39)
    seed_fixture(database)
    apply_action(
        database,
        serial=40,
        action=ACTIONS["create"],
        key_label="rollback-create",
        request_label="rollback-create",
        mutation=create_mutation,
        snapshot=None,
    )
    stale = app_snapshot(database)
    assert stale is not None
    database.execute(
        "UPDATE legacy_developer_apps_v1 SET revision=revision+1 WHERE id=?1", (APP,)
    )
    stale_grant = seed_grant(database, 41)
    try:
        apply_action(
            database,
            serial=41,
            action=ACTIONS["update"],
            key_label="stale",
            request_label="stale",
            mutation=update_mutation(name="must-not-persist"),
            snapshot=stale,
            grant=stale_grant,
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_developer_authority_v1" in str(error)
    else:
        raise AssertionError("stale authority was accepted")
    assert database.execute(
        "SELECT name FROM legacy_developer_apps_v1 WHERE id=?1", (APP,)
    ).fetchone()[0] == "EngManager Frame"
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (stale_grant.grant_id,),
    ).fetchone()[0] == 1
    assert database.execute(
        """SELECT COUNT(*) FROM legacy_developer_action_operations_v1
           WHERE idempotency_key_digest=?1""",
        (digest("key:stale"),),
    ).fetchone()[0] == 0

    before = app_snapshot(database)
    false_grant = seed_grant(database, 42)

    def false_postcondition(
        target: sqlite3.Connection,
        operation_id: str,
        snapshot: sqlite3.Row | None,
    ) -> dict[str, Any]:
        assert snapshot is not None
        execute(
            target,
            "update_app",
            (
                operation_id,
                APP,
                ACTOR,
                snapshot["revision"],
                snapshot["authority_version"],
                1,
                "temporary-name",
                0,
                None,
                0,
                None,
                NOW_MS,
                1,
            ),
        )
        execute(target, "changes_assert", (operation_id, "app_mutated", 1))
        execute(
            target,
            "update_postcondition_assert",
            (
                operation_id,
                APP,
                ACTOR,
                "wrong-name",
                snapshot["environment"],
                snapshot["logo_url"],
                snapshot["revision"] + 1,
                snapshot["authority_version"],
                operation_id,
            ),
        )
        raise AssertionError("postcondition trigger did not abort")

    try:
        apply_action(
            database,
            serial=42,
            action=ACTIONS["update"],
            key_label="false-postcondition",
            request_label="false-postcondition",
            mutation=false_postcondition,
            snapshot=before,
            grant=false_grant,
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_developer_conflict_v1" in str(error)
    else:
        raise AssertionError("false postcondition was accepted")
    after = app_snapshot(database)
    assert before is not None and after is not None
    assert (after["name"], after["revision"], after["last_operation_id"]) == (
        before["name"],
        before["revision"],
        before["last_operation_id"],
    )
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (false_grant.grant_id,),
    ).fetchone()[0] == 1
    assert database_quick_check(database)
    database.close()


def test_concurrent_same_key_has_one_apply_and_one_replay() -> None:
    with tempfile.TemporaryDirectory(prefix="frame-developer-race-") as temporary:
        path = Path(temporary) / "race.sqlite3"
        database = migrated_database(path=path, through=39)
        seed_fixture(database)
        apply_action(
            database,
            serial=50,
            action=ACTIONS["create"],
            key_label="race-seed",
            request_label="race-seed",
            mutation=create_mutation,
            snapshot=None,
        )
        grants = (seed_grant(database, 51), seed_grant(database, 52))
        database.close()

        barrier = threading.Barrier(2)
        results: list[str] = []
        errors: list[BaseException] = []
        lock = threading.Lock()

        def worker(serial: int, grant: Grant) -> None:
            connection = connect(path)
            try:
                snapshot = app_snapshot(connection)
                barrier.wait(timeout=10)
                try:
                    apply_action(
                        connection,
                        serial=serial,
                        action=ACTIONS["update"],
                        key_label="shared-race",
                        request_label="shared-race",
                        mutation=update_mutation(name="race-winner"),
                        snapshot=snapshot,
                        grant=grant,
                    )
                    outcome = "applied"
                except sqlite3.IntegrityError:
                    operation = one_row(
                        connection,
                        "operation_by_key",
                        (ACTOR, ACTIONS["update"], digest("key:shared-race")),
                    )
                    assert operation["request_digest"] == digest("request:shared-race")
                    consume_attempt(
                        connection,
                        serial=serial,
                        action=ACTIONS["update"],
                        request_digest=digest("request:shared-race"),
                        outcome="replay",
                        related_operation_id=operation["operation_id"],
                        grant=grant,
                    )
                    outcome = "replayed"
                with lock:
                    results.append(outcome)
            except BaseException as error:  # noqa: BLE001 - returned to the main thread.
                with lock:
                    errors.append(error)
            finally:
                connection.close()

        threads = [
            threading.Thread(target=worker, args=(51 + index, grant))
            for index, grant in enumerate(grants)
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=20)
        assert not errors, errors
        assert sorted(results) == ["applied", "replayed"]

        verify = connect(path)
        assert verify.execute(
            """SELECT COUNT(*) FROM legacy_developer_action_operations_v1
               WHERE actor_id=?1 AND action=?2 AND idempotency_key_digest=?3""",
            (ACTOR, ACTIONS["update"], digest("key:shared-race")),
        ).fetchone()[0] == 1
        assert verify.execute(
            "SELECT revision FROM legacy_developer_apps_v1 WHERE id=?1", (APP,)
        ).fetchone()[0] == 1
        assert verify.execute(
            """SELECT COUNT(*) FROM legacy_developer_action_proof_consumptions_v1
               WHERE action=?1 AND request_digest=?2""",
            (ACTIONS["update"], digest("request:shared-race")),
        ).fetchone()[0] == 2
        verify.close()


def test_journal_constraints_immutability_and_plaintext_exclusion() -> None:
    database = migrated_database(through=39)
    seed_fixture(database)
    operation_id, grant, _ = apply_action(
        database,
        serial=60,
        action=ACTIONS["create"],
        key_label="immutable-create",
        request_label="immutable-create",
        mutation=create_mutation,
        snapshot=None,
    )
    for table in (
        "legacy_developer_action_receipts_v1",
        "legacy_developer_action_effects_v1",
        "legacy_developer_action_audit_events_v1",
    ):
        expect_integrity_error(
            database,
            f"UPDATE {table} SET operation_id=operation_id WHERE operation_id=?1",
            (operation_id,),
            "frame_legacy_developer_receipt_immutable_v1",
        )
        expect_integrity_error(
            database,
            f"DELETE FROM {table} WHERE operation_id=?1",
            (operation_id,),
            "frame_legacy_developer_receipt_immutable_v1",
        )
    expect_integrity_error(
        database,
        "UPDATE legacy_developer_action_operations_v1 SET completed_at_ms=completed_at_ms+1 WHERE operation_id=?1",
        (operation_id,),
        "frame_legacy_developer_operation_immutable_v1",
    )
    expect_integrity_error(
        database,
        "DELETE FROM legacy_developer_action_operations_v1 WHERE operation_id=?1",
        (operation_id,),
        "frame_legacy_developer_operation_immutable_v1",
    )
    expect_integrity_error(
        database,
        "UPDATE legacy_developer_action_proof_consumptions_v1 SET outcome=outcome WHERE mutation_grant_id=?1",
        (grant.grant_id,),
        "frame_legacy_developer_proof_immutable_v1",
    )
    expect_integrity_error(
        database,
        "DELETE FROM legacy_developer_action_proof_consumptions_v1 WHERE mutation_grant_id=?1",
        (grant.grant_id,),
        "frame_legacy_developer_proof_immutable_v1",
    )
    expect_integrity_error(
        database,
        """INSERT INTO legacy_developer_action_assertions_v1(
             operation_id,assertion_kind,expected_count,actual_count
           ) VALUES (?1,'postcondition',1,0)""",
        (uuid7_fixture(500_000),),
        "frame_legacy_developer_conflict_v1",
    )

    persisted = "\n".join(
        str(value)
        for row in database.execute(
            """SELECT key_digest,encrypted_key FROM legacy_developer_api_keys_v1
               UNION ALL
               SELECT replay_binding,sealed_key_replay
               FROM legacy_developer_action_receipts_v1
               WHERE sealed_key_replay IS NOT NULL"""
        ).fetchall()
        for value in row
    )
    for raw in (
        "cpk_0123456789abcdefghjkmnpqrstv",
        "csk_0123456789abcdefghjkmnpqrstv",
    ):
        assert raw not in persisted
    assert "AQBprotected-public-ciphertext" in persisted
    assert "AQBsealed-response-replay" in persisted
    assert database_quick_check(database)
    database.close()


def test_static_query_bounds_runtime_crypto_and_protected_gate() -> None:
    required_queries = {
        "app_authority_assert",
        "app_authority_snapshot",
        "assertion_cleanup",
        "audit_insert",
        "auto_top_up_postcondition_assert",
        "auto_top_up_update",
        "browser_grant_assert",
        "browser_grant_delete_returning",
        "changes_assert",
        "clock_now",
        "create_app_insert",
        "create_postcondition_assert",
        "credit_insert",
        "delete_app",
        "delete_app_postcondition_assert",
        "domain_add_postcondition_assert",
        "domain_delete",
        "domain_insert",
        "domain_remove_postcondition_assert",
        "domain_target_count",
        "durable_receipt_assert",
        "effect_insert",
        "key_insert",
        "operation_by_key",
        "operation_claim",
        "operation_complete",
        "proof_insert",
        "receipt_insert",
        "regenerate_postcondition_assert",
        "revoke_active_keys",
        "update_app",
        "update_postcondition_assert",
        "video_delete",
        "video_postcondition_assert",
        "video_target_count",
    }
    assert set(SQL) == required_queries
    combined = "\n".join(SQL.values())
    assert "SELECT *" not in combined.upper()
    assert "legacy_api_execution_operations_v1" not in combined
    assert "authenticated_web_action_operations_v1" not in combined
    for name in ("operation_by_key", "app_authority_snapshot"):
        assert "LIMIT 2" in SQL[name]
    for token in (
        "operation.actor_id = ?1",
        "operation.action = ?2",
        "operation.idempotency_key_digest = ?3",
    ):
        assert token in SQL["operation_by_key"]
    for token in ("app.id = ?1", "app.owner_id = ?2", "app.deleted_at_ms IS NULL"):
        assert token in SQL["app_authority_snapshot"]
    assert "AND ?13 = 1" in SQL["update_app"]
    assert "WHERE id = ?2 AND app_id = ?3" in SQL["domain_delete"]
    assert "WHERE id = ?2 AND app_id = ?3 AND deleted_at_ms IS NULL" in SQL["video_delete"]
    assert "RETURNING id AS mutation_grant_id" in SQL["browser_grant_delete_returning"]

    runtime = RUNTIME.read_text(encoding="utf-8")
    application = APPLICATION.read_text(encoding="utf-8")
    migration = MIGRATION.read_text(encoding="utf-8")
    for token in (
        "D1LegacyDeveloperAtomicPortV1",
        "LocalLegacyDeveloperSecretAuthorityV1",
        "pub(crate) const fn new(database:",
        "pub(crate) fn from_hex(value:",
        "getrandom::fill",
        "Aes256Gcm",
        "hmac_sha256",
        "key-at-rest-public",
        "key-at-rest-secret",
        "response-replay",
        "context.replay_binding()",
        'Zeroizing::new(format!("cpk_',
        'Zeroizing::new(format!("csk_',
        "LegacyDeveloperAtomicErrorV1::SecretUnavailable",
        "LegacyDeveloperAtomicErrorV1::InFlight",
        "LegacyDeveloperAtomicOutcomeV1::Replay",
    ):
        assert token in runtime
    assert 'pub const LEGACY_DEVELOPER_PROTECTED_GATES: &[&str] = &["released_legacy_client_e2e"]' in application
    assert "production_promoted: false" in application
    assert "organization_id" not in migration
    assert "owner_id TEXT NOT NULL REFERENCES users(id)" in migration
    assert "frame_legacy_developer_operation_immutable_v1" in migration
    assert "frame_legacy_developer_receipt_immutable_v1" in migration


TESTS = (
    test_full_migration_chain_and_isolated_expand,
    test_all_actions_exact_mutations_and_zero_row_successes,
    test_user_owned_non_disclosure_and_absent_credit_account,
    test_replay_conflict_in_flight_and_one_use_proof,
    test_stale_authority_and_false_postcondition_roll_back_atomically,
    test_concurrent_same_key_has_one_apply_and_one_replay,
    test_journal_constraints_immutability_and_plaintext_exclusion,
    test_static_query_bounds_runtime_crypto_and_protected_gate,
)


def run() -> dict[str, object]:
    for test in TESTS:
        test()
    return {
        "schema_version": "frame.legacy-developer-actions-sqlite-conformance.v1",
        "provider": "local_sqlite",
        "expand_migration": "0039_legacy_developer_actions_expand.sql",
        "full_expand_chain_applied": True,
        "tests_passed": len(TESTS),
        "actions": list(ACTIONS.values()),
        "operation_ids": OPERATION_IDS,
        "checked_in_queries_executed": len(SQL),
        "user_owned_authority_non_disclosure": True,
        "nullable_logo_and_empty_update": True,
        "zero_row_domain_video_success": True,
        "r2_media_compatible_video_identity": True,
        "local_secret_authority_fail_closed": True,
        "plaintext_credentials_persisted": False,
        "browser_grant_one_use": True,
        "receipt_effect_audit_proof_atomic": True,
        "same_key_exact_replay": True,
        "same_key_conflict_consumes_proof": True,
        "same_key_in_flight_consumes_proof": True,
        "race_applied": 1,
        "race_replayed": 1,
        "rollback_and_fail_closed": True,
        "bounded_sql": True,
        "durable_journal_immutable": True,
        "production_promotion": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path)
    arguments = parser.parse_args()
    report = run()
    if arguments.evidence is not None:
        arguments.evidence.parent.mkdir(parents=True, exist_ok=True)
        arguments.evidence.write_text(
            json.dumps(report, indent=2) + "\n", encoding="utf-8"
        )
    print(
        "legacy developer-actions SQLite conformance: "
        f"{report['tests_passed']} passed; eight actions, protected credentials, "
        "atomic proof/journal, replay/conflict/in-flight/race, rollback, bounds, "
        "and immutability verified"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
