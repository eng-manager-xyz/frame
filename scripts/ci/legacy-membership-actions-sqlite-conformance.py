#!/usr/bin/env python3
"""SQLite proof for the six source-pinned legacy membership mutations.

The suite applies the complete expand chain through migration 0040 and executes
the checked-in query files in the same order as the D1 atomic port. It proves
tenant authority, exact typed postconditions, creator forcing, full affected-
subject generation/revocation, one-use browser proofs, replay/conflict behavior,
rollback, immutability, and the 100000 discovered-member bound.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sqlite3
import tempfile
import threading
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable


ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_membership_actions"
RUNTIME = ROOT / "apps/control-plane/src/legacy_membership_actions_runtime.rs"
APPLICATION = ROOT / "crates/application/src/legacy_membership_actions.rs"

NOW_MS = 1_700_000_000_000
REMOVE_ACTION = "legacy.membership.remove_organization_invite"
ADD_ACTION = "legacy.membership.add_space_member"
ADD_MEMBERS_ACTION = "legacy.membership.add_space_members"
BATCH_REMOVE_ACTION = "legacy.membership.batch_remove_space_members"
REMOVE_MEMBER_ACTION = "legacy.membership.remove_space_member"
SET_ACTION = "legacy.membership.set_space_members"


def fixture_uuid(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


OWNER = fixture_uuid(1)
ADMIN = fixture_uuid(2)
CREATOR = fixture_uuid(3)
MANAGER = fixture_uuid(4)
TARGET = fixture_uuid(5)
TARGET_TWO = fixture_uuid(6)
OLD_MEMBER = fixture_uuid(7)
ORDINARY = fixture_uuid(8)
OUTSIDER = fixture_uuid(9)
ORGANIZATION = fixture_uuid(20)
FOREIGN_ORGANIZATION = fixture_uuid(21)
SPACE = fixture_uuid(30)
SPACE_TWO = fixture_uuid(31)
FOREIGN_SPACE = fixture_uuid(32)
INVITE = fixture_uuid(40)
FOREIGN_INVITE = fixture_uuid(41)


def legacy_id(number: int) -> str:
    return f"{number:015d}"


USER_LEGACY_IDS = {
    user_id: legacy_id(1_000 + index)
    for index, user_id in enumerate(
        (OWNER, ADMIN, CREATOR, MANAGER, TARGET, TARGET_TWO, OLD_MEMBER, ORDINARY, OUTSIDER)
    )
}

SEEDED_MEMBER_ALIASES = {
    (space_id, user_id): (
        legacy_id(2_000 + index),
        fixture_uuid(50_000 + index),
    )
    for index, (space_id, user_id) in enumerate(
        (
            (SPACE, CREATOR),
            (SPACE, MANAGER),
            (SPACE, OLD_MEMBER),
            (SPACE_TWO, CREATOR),
            (FOREIGN_SPACE, OUTSIDER),
        )
    )
}


SQL = {
    path.stem: path.read_text(encoding="utf-8").strip()
    for path in sorted(QUERIES.glob("*.sql"))
}


def sha256(label: str) -> str:
    return hashlib.sha256(f"frame-membership-conformance:{label}".encode()).hexdigest()


def migration_paths(through: int = 40) -> list[Path]:
    return [
        path
        for path in sorted(MIGRATIONS.glob("*.sql"))
        if int(path.name[:4]) <= through
    ]


def connect(path: Path | None = None) -> sqlite3.Connection:
    database = sqlite3.connect(
        ":memory:" if path is None else path,
        timeout=20,
        isolation_level=None,
        check_same_thread=False,
    )
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    database.execute("PRAGMA busy_timeout = 20000")
    if path is not None:
        database.execute("PRAGMA journal_mode = WAL")
    return database


def migrated_database(
    *, path: Path | None = None, through: int = 40
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


def seed_user(
    database: sqlite3.Connection,
    user_id: str,
    label: str,
    active_organization: str,
    *,
    status: str = "active",
    deleted_at_ms: int | None = None,
) -> None:
    database.execute(
        """INSERT INTO users(
             id,email,display_name,created_at_ms,updated_at_ms,status,
             deleted_at_ms,active_organization_id,organization_preference_revision
           ) VALUES (?1,?2,?3,1,1,?4,?5,?6,7)""",
        (
            user_id,
            f"{label}@example.invalid",
            label,
            status,
            deleted_at_ms,
            active_organization,
        ),
    )
    database.execute(
        """INSERT INTO auth_identities_v2(
             user_id,identity_revision,session_version,created_at_ms,
             updated_at_ms,revision
           ) VALUES (?1,1,3,1,1,0)""",
        (user_id,),
    )


def seed_fixture(database: sqlite3.Connection) -> None:
    database.execute("BEGIN")
    try:
        for user_id, label, active_org in (
            (OWNER, "owner", ORGANIZATION),
            (ADMIN, "admin", ORGANIZATION),
            (CREATOR, "creator", ORGANIZATION),
            (MANAGER, "manager", ORGANIZATION),
            (TARGET, "target", ORGANIZATION),
            (TARGET_TWO, "target-two", ORGANIZATION),
            (OLD_MEMBER, "old", ORGANIZATION),
            (ORDINARY, "ordinary", ORGANIZATION),
            (OUTSIDER, "outsider", FOREIGN_ORGANIZATION),
        ):
            seed_user(database, user_id, label, active_org)

        database.executemany(
            """INSERT INTO organizations(
                 id,owner_id,name,status,created_at_ms,updated_at_ms,
                 revision,authority_version
               ) VALUES (?1,?2,?3,'active',1,1,11,13)""",
            (
                (ORGANIZATION, OWNER, "primary"),
                (FOREIGN_ORGANIZATION, OUTSIDER, "foreign"),
            ),
        )
        database.executemany(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,created_at_ms,updated_at_ms,
                 revision,authority_version
               ) VALUES (?1,?2,?3,'active',1,1,5,9)""",
            (
                (ORGANIZATION, OWNER, "owner"),
                (ORGANIZATION, ADMIN, "admin"),
                (ORGANIZATION, CREATOR, "member"),
                (ORGANIZATION, MANAGER, "member"),
                (ORGANIZATION, TARGET, "member"),
                (ORGANIZATION, TARGET_TWO, "member"),
                (ORGANIZATION, OLD_MEMBER, "member"),
                (ORGANIZATION, ORDINARY, "viewer"),
                (FOREIGN_ORGANIZATION, OUTSIDER, "owner"),
            ),
        )
        database.executemany(
            """INSERT INTO spaces(
                 id,organization_id,created_by_user_id,name,created_at_ms,
                 updated_at_ms,revision,authority_version
               ) VALUES (?1,?2,?3,?4,1,1,17,19)""",
            (
                (SPACE, ORGANIZATION, CREATOR, "primary"),
                (SPACE_TWO, ORGANIZATION, CREATOR, "second"),
                (FOREIGN_SPACE, FOREIGN_ORGANIZATION, OUTSIDER, "foreign"),
            ),
        )
        database.executemany(
            """INSERT INTO space_members(
                 space_id,user_id,role,created_at_ms,updated_at_ms,state,revision
               ) VALUES (?1,?2,?3,1,1,'active',3)""",
            (
                (SPACE, CREATOR, "manager"),
                (SPACE, MANAGER, "manager"),
                (SPACE, OLD_MEMBER, "viewer"),
                (SPACE_TWO, CREATOR, "manager"),
                (FOREIGN_SPACE, OUTSIDER, "manager"),
            ),
        )
        database.executemany(
            """INSERT INTO organization_invites(
                 id,organization_id,invited_email_digest,invited_by_user_id,
                 role,status,token_digest,created_at_ms,expires_at_ms,revision
               ) VALUES (?1,?2,?3,?4,'member','pending',?5,1,9999999999999,0)""",
            (
                (INVITE, ORGANIZATION, sha256("invite-email"), OWNER, sha256("invite-token")),
                (
                    FOREIGN_INVITE,
                    FOREIGN_ORGANIZATION,
                    sha256("foreign-email"),
                    OUTSIDER,
                    sha256("foreign-token"),
                ),
            ),
        )
        alias_table_exists = database.execute(
            "SELECT 1 FROM sqlite_master WHERE type='table' "
            "AND name='legacy_space_member_aliases_v1'"
        ).fetchone()
        if alias_table_exists is not None:
            database.executemany(
                """INSERT INTO legacy_space_member_aliases_v1(
                     mapped_member_id,legacy_member_id,legacy_user_id,
                     space_id,user_id,created_at_ms
                   ) VALUES (?1,?2,?3,?4,?5,1)""",
                (
                    (
                        mapped_member_id,
                        legacy_member_id,
                        USER_LEGACY_IDS[user_id],
                        space_id,
                        user_id,
                    )
                    for (space_id, user_id), (
                        legacy_member_id,
                        mapped_member_id,
                    ) in SEEDED_MEMBER_ALIASES.items()
                ),
            )
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


@dataclass(frozen=True)
class Grant:
    grant_id: str
    session_id: str
    user_id: str


def seed_grant(database: sqlite3.Connection, serial: int, user_id: str) -> Grant:
    base = 10_000 + serial * 5
    session_id = fixture_uuid(base)
    family_id = fixture_uuid(base + 1)
    grant_id = fixture_uuid(base + 2)
    token_digest = sha256(f"token:{serial}:{user_id}")
    database.execute(
        """INSERT INTO auth_sessions_v2(
             id,family_id,user_id,client_kind,token_key_version,token_digest,
             csrf_key_version,csrf_digest,browser_origin,issued_at_ms,rotated_at_ms,
             idle_expires_at_ms,absolute_expires_at_ms,session_version,generation,
             state,revision,last_operation_id
           ) VALUES (
             ?1,?2,?3,'browser',7,?4,7,?5,'https://frame.engmanager.xyz',
             ?6,?6,?7,?8,3,4,'active',0,?9
           )""",
        (
            session_id,
            family_id,
            user_id,
            token_digest,
            sha256(f"csrf:{serial}"),
            NOW_MS - 1000,
            8_000_000_000_000_000,
            8_500_000_000_000_000,
            fixture_uuid(base + 3),
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
            user_id,
            token_digest,
            NOW_MS - 500,
            fixture_uuid(base + 4),
        ),
    )
    return Grant(grant_id, session_id, user_id)


def one_row(
    database: sqlite3.Connection, sql: str, parameters: tuple[object, ...]
) -> sqlite3.Row:
    rows = database.execute(sql, parameters).fetchall()
    if len(rows) != 1:
        raise AssertionError(f"expected one bounded row, received {len(rows)}")
    return rows[0]


def authority_snapshot(
    database: sqlite3.Connection,
    *,
    actor_id: str,
    organization_id: str,
    space_id: str | None,
) -> sqlite3.Row:
    if space_id is None:
        return one_row(
            database,
            SQL["invite_authority_snapshot"],
            (actor_id, organization_id),
        )
    return one_row(
        database,
        SQL["space_authority_snapshot"],
        (actor_id, organization_id, space_id),
    )


def tenant_authority_snapshot(
    database: sqlite3.Connection, *, actor_id: str, organization_id: str
) -> sqlite3.Row:
    return one_row(
        database,
        SQL["tenant_authority_snapshot"],
        (actor_id, organization_id),
    )


def assert_authority(
    database: sqlite3.Connection,
    operation_id: str,
    actor_id: str,
    organization_id: str,
    snapshot: sqlite3.Row,
) -> None:
    if "space_id" not in snapshot.keys():
        database.execute(
            SQL["invite_authority_assert"],
            (
                operation_id,
                actor_id,
                organization_id,
                snapshot["selection_revision"],
                snapshot["organization_revision"],
                snapshot["organization_authority_version"],
                snapshot["membership_role"],
                snapshot["membership_state"],
                snapshot["membership_revision"],
                snapshot["membership_authority_version"],
                snapshot["actor_authority"],
            ),
        )
        return
    database.execute(
        SQL["space_authority_assert"],
        (
            operation_id,
            actor_id,
            organization_id,
            snapshot["selection_revision"],
            snapshot["organization_revision"],
            snapshot["organization_authority_version"],
            snapshot["membership_role"],
            snapshot["membership_state"],
            snapshot["membership_revision"],
            snapshot["membership_authority_version"],
            snapshot["space_id"],
            snapshot["creator_id"],
            snapshot["space_revision"],
            snapshot["space_authority_version"],
            snapshot["space_membership_role"],
            snapshot["space_membership_state"],
            snapshot["space_membership_revision"],
            snapshot["actor_authority"],
        ),
    )


def assert_tenant_authority(
    database: sqlite3.Connection,
    operation_id: str,
    actor_id: str,
    organization_id: str,
    snapshot: sqlite3.Row,
) -> None:
    database.execute(
        SQL["tenant_authority_assert"],
        (
            operation_id,
            actor_id,
            organization_id,
            snapshot["selection_revision"],
            snapshot["organization_revision"],
            snapshot["organization_authority_version"],
            snapshot["membership_role"],
            snapshot["membership_state"],
            snapshot["membership_revision"],
            snapshot["membership_authority_version"],
            snapshot["actor_authority"],
        ),
    )


def change_assert(
    database: sqlite3.Connection,
    operation_id: str,
    kind: str,
    expected: int,
) -> None:
    database.execute(SQL["changes_assert"], (operation_id, kind, expected))


def assert_grant(
    database: sqlite3.Connection, operation_id: str, grant: Grant
) -> None:
    database.execute(
        SQL["browser_grant_assert"],
        (operation_id, grant.grant_id, grant.session_id, grant.user_id, NOW_MS),
    )


def consume_grant(
    database: sqlite3.Connection, operation_id: str, grant: Grant
) -> sqlite3.Row:
    row = one_row(
        database,
        SQL["browser_grant_delete_returning"],
        (grant.grant_id, grant.session_id, grant.user_id),
    )
    change_assert(database, operation_id, "grant_consumed", 1)
    assert tuple(row) == (grant.grant_id, grant.session_id, grant.user_id)
    return row


@dataclass(frozen=True)
class Plan:
    operation_id: str
    audit_id: str
    organization_id: str
    actor_id: str
    action: str
    key_digest: str
    request_digest: str
    grant: Grant
    space_id: str | None = None
    creator_id: str | None = None
    invite_id: str | None = None


def plan(
    serial: int,
    *,
    actor_id: str,
    action: str,
    grant: Grant,
    organization_id: str = ORGANIZATION,
    space_id: str | None = None,
    creator_id: str | None = None,
    invite_id: str | None = None,
    key_digest: str | None = None,
    request_digest: str | None = None,
) -> Plan:
    return Plan(
        operation_id=fixture_uuid(30_000 + serial * 3),
        audit_id=fixture_uuid(30_000 + serial * 3 + 1),
        organization_id=organization_id,
        actor_id=actor_id,
        action=action,
        key_digest=key_digest or sha256(f"key:{serial}"),
        request_digest=request_digest or sha256(f"request:{serial}"),
        grant=grant,
        space_id=space_id,
        creator_id=creator_id,
        invite_id=invite_id,
    )


def generated_member_alias(
    action_plan: Plan, user_id: str, ordinal: int
) -> tuple[str, str]:
    label = f"{action_plan.operation_id}:{user_id}:{ordinal}"
    legacy_member_id = hashlib.sha256(label.encode()).hexdigest()[:15]
    mapped_member_id = str(uuid.uuid5(uuid.NAMESPACE_URL, f"frame-membership:{label}"))
    return legacy_member_id, mapped_member_id


def final_members_payload(
    action_plan: Plan, members: list[tuple[str, str]]
) -> str:
    payload = []
    for ordinal, (user_id, role) in enumerate(members):
        legacy_member_id, mapped_member_id = generated_member_alias(
            action_plan, user_id, ordinal
        )
        payload.append(
            {
                "userId": user_id,
                "legacyUserId": USER_LEGACY_IDS[user_id],
                "legacyMemberId": legacy_member_id,
                "mappedMemberId": mapped_member_id,
                "role": role,
            }
        )
    return json.dumps(payload, separators=(",", ":"))


def member_id_payload(member_ids: list[tuple[str, str]]) -> str:
    return json.dumps(
        [
            {"legacyMemberId": legacy_member_id, "mappedMemberId": mapped_member_id}
            for legacy_member_id, mapped_member_id in member_ids
        ],
        separators=(",", ":"),
    )


def claim(
    database: sqlite3.Connection, action_plan: Plan, snapshot: sqlite3.Row
) -> None:
    database.execute(
        SQL["operation_claim"],
        (
            action_plan.operation_id,
            action_plan.organization_id,
            action_plan.actor_id,
            action_plan.action,
            action_plan.key_digest,
            action_plan.request_digest,
            NOW_MS,
        ),
    )
    assert_authority(
        database,
        action_plan.operation_id,
        action_plan.actor_id,
        action_plan.organization_id,
        snapshot,
    )
    assert_grant(database, action_plan.operation_id, action_plan.grant)


def graph_assertions(database: sqlite3.Connection, action_plan: Plan) -> None:
    if action_plan.creator_id is None:
        raise AssertionError("space plan has no creator")
    database.execute(
        SQL["target_graph_assert"],
        (action_plan.operation_id, action_plan.organization_id),
    )
    database.execute(
        SQL["creator_graph_assert"],
        (
            action_plan.operation_id,
            action_plan.organization_id,
            action_plan.creator_id,
        ),
    )


def authority_side_effects(
    database: sqlite3.Connection,
    action_plan: Plan,
    *,
    subject_query: str = "authority_subject_insert",
) -> None:
    database.execute(
        SQL[subject_query],
        (action_plan.operation_id, action_plan.organization_id),
    )
    database.execute(
        SQL["authority_generation_upsert"],
        (action_plan.operation_id, action_plan.organization_id, NOW_MS),
    )
    database.execute(
        SQL["authority_generation_changes_assert"], (action_plan.operation_id,)
    )
    database.execute(
        SQL["authority_generation_postcondition_assert"],
        (action_plan.operation_id, action_plan.organization_id),
    )
    database.execute(
        SQL["revoked_grant_snapshot_insert"], (action_plan.operation_id,)
    )
    database.execute(SQL["revoke_grants"], (action_plan.operation_id,))
    database.execute(
        SQL["revoked_grant_changes_assert"], (action_plan.operation_id,)
    )
    database.execute(
        SQL["revoked_grant_postcondition_assert"], (action_plan.operation_id,)
    )


def insert_effect(database: sqlite3.Connection, action_plan: Plan) -> None:
    if action_plan.action == REMOVE_ACTION:
        flags = (1, 0, 0, 0)
        path = "/dashboard/settings/organization"
    else:
        flags = (0, 1, 1, 1)
        path = f"/dashboard/spaces/{action_plan.space_id}"
    database.execute(
        SQL["effect_insert"],
        (
            action_plan.operation_id,
            action_plan.organization_id,
            action_plan.space_id,
            *flags,
            path,
            NOW_MS,
        ),
    )
    change_assert(database, action_plan.operation_id, "effect_inserted", 1)


def finish(database: sqlite3.Connection, action_plan: Plan) -> None:
    database.execute(
        SQL["audit_insert"],
        (
            action_plan.audit_id,
            action_plan.operation_id,
            action_plan.organization_id,
            action_plan.actor_id,
            action_plan.action,
            sha256(f"principal:{action_plan.actor_id}"),
            sha256(f"mutation:{action_plan.request_digest}"),
            NOW_MS,
        ),
    )
    change_assert(database, action_plan.operation_id, "audit_inserted", 1)
    database.execute(
        SQL["proof_insert"],
        (
            action_plan.grant.grant_id,
            action_plan.grant.session_id,
            action_plan.grant.user_id,
            action_plan.operation_id,
            action_plan.organization_id,
            action_plan.action,
            action_plan.request_digest,
            "applied",
            NOW_MS,
        ),
    )
    change_assert(database, action_plan.operation_id, "proof_journaled", 1)
    database.execute(
        SQL["operation_complete"], (action_plan.operation_id, NOW_MS)
    )
    change_assert(database, action_plan.operation_id, "operation_complete", 1)
    database.execute(
        SQL["durable_receipt_assert"],
        (
            action_plan.operation_id,
            action_plan.organization_id,
            action_plan.actor_id,
            action_plan.action,
            action_plan.request_digest,
            action_plan.grant.grant_id,
            action_plan.grant.session_id,
            "applied",
        ),
    )
    database.execute(SQL["assertion_cleanup"], (action_plan.operation_id,))


def transaction(database: sqlite3.Connection, callback: Callable[[], None]) -> None:
    database.execute("BEGIN IMMEDIATE")
    try:
        callback()
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")


def apply_remove(database: sqlite3.Connection, action_plan: Plan) -> None:
    snapshot = authority_snapshot(
        database,
        actor_id=action_plan.actor_id,
        organization_id=action_plan.organization_id,
        space_id=None,
    )

    def body() -> None:
        claim(database, action_plan, snapshot)
        database.execute(
            SQL["invite_target_assert"],
            (
                action_plan.operation_id,
                action_plan.invite_id,
                action_plan.organization_id,
            ),
        )
        consume_grant(database, action_plan.operation_id, action_plan.grant)
        database.execute(
            SQL["invite_delete"],
            (action_plan.invite_id, action_plan.organization_id),
        )
        change_assert(database, action_plan.operation_id, "mutation_rows", 1)
        database.execute(
            SQL["invite_postcondition_assert"],
            (
                action_plan.operation_id,
                action_plan.invite_id,
                action_plan.organization_id,
            ),
        )
        database.execute(
            SQL["receipt_insert_invite"],
            (
                action_plan.operation_id,
                action_plan.invite_id,
                snapshot["actor_authority"],
                NOW_MS,
            ),
        )
        change_assert(database, action_plan.operation_id, "receipt_inserted", 1)
        insert_effect(database, action_plan)
        finish(database, action_plan)

    transaction(database, body)


def apply_add(
    database: sqlite3.Connection,
    action_plan: Plan,
    *,
    target_id: str,
    role: str,
) -> None:
    if action_plan.space_id is None:
        raise AssertionError("add plan has no space")
    snapshot = authority_snapshot(
        database,
        actor_id=action_plan.actor_id,
        organization_id=action_plan.organization_id,
        space_id=action_plan.space_id,
    )

    def body() -> None:
        claim(database, action_plan, snapshot)
        database.execute(
            SQL["final_members_insert"],
            (
                action_plan.operation_id,
                final_members_payload(action_plan, [(target_id, role)]),
            ),
        )
        graph_assertions(database, action_plan)
        database.execute(
            SQL["add_absent_assert"],
            (action_plan.operation_id, action_plan.space_id, target_id),
        )
        consume_grant(database, action_plan.operation_id, action_plan.grant)
        database.execute(
            SQL["add_insert"],
            (action_plan.operation_id, action_plan.space_id, NOW_MS),
        )
        change_assert(database, action_plan.operation_id, "members_inserted", 1)
        database.execute(
            SQL["member_alias_insert_added"],
            (action_plan.operation_id, action_plan.space_id, NOW_MS),
        )
        change_assert(database, action_plan.operation_id, "aliases_inserted", 1)
        database.execute(
            SQL["member_alias_postcondition_assert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        database.execute(
            SQL["add_postcondition_assert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        database.execute(
            SQL["out_of_scope_assert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        authority_side_effects(database, action_plan)
        database.execute(
            SQL["receipt_insert_add"],
            (
                action_plan.operation_id,
                action_plan.space_id,
                action_plan.creator_id,
                snapshot["actor_authority"],
                NOW_MS,
            ),
        )
        change_assert(database, action_plan.operation_id, "receipt_inserted", 1)
        insert_effect(database, action_plan)
        finish(database, action_plan)

    transaction(database, body)


def apply_bulk_add(
    database: sqlite3.Connection,
    action_plan: Plan,
    *,
    members: list[tuple[str, str]],
) -> None:
    if action_plan.space_id is None or action_plan.creator_id is None:
        raise AssertionError("bulk-add plan has no space/creator")
    snapshot = authority_snapshot(
        database,
        actor_id=action_plan.actor_id,
        organization_id=action_plan.organization_id,
        space_id=action_plan.space_id,
    )
    targets_json = final_members_payload(action_plan, members)

    def body() -> None:
        claim(database, action_plan, snapshot)
        database.execute(
            SQL["bulk_add_duplicate_assert"],
            (action_plan.operation_id, targets_json, action_plan.space_id),
        )
        database.execute(
            SQL["final_members_insert"],
            (action_plan.operation_id, targets_json),
        )
        graph_assertions(database, action_plan)
        database.execute(
            SQL["previous_snapshot_insert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        database.execute(SQL["previous_bound_assert"], (action_plan.operation_id,))
        database.execute(
            SQL["previous_aliases_complete_assert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        consume_grant(database, action_plan.operation_id, action_plan.grant)
        database.execute(
            SQL["bulk_add_insert"],
            (action_plan.operation_id, action_plan.space_id, NOW_MS),
        )
        database.execute(SQL["bulk_added_changes_assert"], (action_plan.operation_id,))
        database.execute(
            SQL["member_alias_insert_added"],
            (action_plan.operation_id, action_plan.space_id, NOW_MS),
        )
        database.execute(SQL["aliases_added_changes_assert"], (action_plan.operation_id,))
        database.execute(
            SQL["bulk_add_postcondition_assert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        database.execute(
            SQL["member_alias_added_postcondition_assert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        database.execute(
            SQL["out_of_scope_assert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        authority_side_effects(
            database,
            action_plan,
            subject_query="authority_subject_insert_added",
        )
        database.execute(
            SQL["receipt_insert_bulk_add"],
            (
                action_plan.operation_id,
                action_plan.space_id,
                action_plan.creator_id,
                snapshot["actor_authority"],
                NOW_MS,
            ),
        )
        change_assert(database, action_plan.operation_id, "receipt_inserted", 1)
        database.execute(
            SQL["effect_insert_bulk_add"],
            (
                action_plan.operation_id,
                action_plan.organization_id,
                action_plan.space_id,
                f"/dashboard/spaces/{action_plan.space_id}",
                NOW_MS,
            ),
        )
        change_assert(database, action_plan.operation_id, "effect_inserted", 1)
        finish(database, action_plan)

    transaction(database, body)


def apply_member_removal(
    database: sqlite3.Connection,
    action_plan: Plan,
    *,
    member_ids: list[tuple[str, str]],
    single: bool,
) -> None:
    targets_json = member_id_payload(member_ids)
    discovered = database.execute(SQL["member_alias_targets"], (targets_json,)).fetchall()
    active = [
        row
        for row in discovered
        if row["removed_at_ms"] is None
        and row["role"] is not None
        and row["state"] == "active"
        and row["revision"] is not None
    ]
    spaces = {row["space_id"] for row in active}
    if len(spaces) > 1:
        raise RuntimeError("cross-space removal rejected before mutation")
    if single and len(active) != 1:
        raise LookupError("single member target missing")

    if not active:
        if single or action_plan.action != BATCH_REMOVE_ACTION:
            raise LookupError("member target missing")
        snapshot = tenant_authority_snapshot(
            database,
            actor_id=action_plan.actor_id,
            organization_id=action_plan.organization_id,
        )

        def noop_body() -> None:
            database.execute(
                SQL["operation_claim"],
                (
                    action_plan.operation_id,
                    action_plan.organization_id,
                    action_plan.actor_id,
                    action_plan.action,
                    action_plan.key_digest,
                    action_plan.request_digest,
                    NOW_MS,
                ),
            )
            assert_tenant_authority(
                database,
                action_plan.operation_id,
                action_plan.actor_id,
                action_plan.organization_id,
                snapshot,
            )
            assert_grant(database, action_plan.operation_id, action_plan.grant)
            database.execute(
                SQL["removal_no_match_assert"],
                (action_plan.operation_id, targets_json),
            )
            consume_grant(database, action_plan.operation_id, action_plan.grant)
            database.execute(
                SQL["receipt_insert_batch_remove"],
                (
                    action_plan.operation_id,
                    None,
                    None,
                    snapshot["actor_authority"],
                    0,
                    NOW_MS,
                ),
            )
            change_assert(database, action_plan.operation_id, "receipt_inserted", 1)
            database.execute(
                SQL["effect_insert"],
                (
                    action_plan.operation_id,
                    action_plan.organization_id,
                    None,
                    0,
                    0,
                    0,
                    0,
                    "",
                    NOW_MS,
                ),
            )
            change_assert(database, action_plan.operation_id, "effect_inserted", 1)
            finish(database, action_plan)

        transaction(database, noop_body)
        return

    space_id = next(iter(spaces))
    if action_plan.space_id != space_id or action_plan.creator_id is None:
        raise AssertionError("removal plan does not match discovered space")
    snapshot = authority_snapshot(
        database,
        actor_id=action_plan.actor_id,
        organization_id=action_plan.organization_id,
        space_id=space_id,
    )

    def body() -> None:
        claim(database, action_plan, snapshot)
        database.execute(
            SQL["removal_targets_insert"],
            (action_plan.operation_id, targets_json, space_id),
        )
        database.execute(
            SQL["removal_targets_assert"],
            (action_plan.operation_id, 1),
        )
        database.execute(
            SQL["removed_target_graph_assert"],
            (action_plan.operation_id, action_plan.organization_id),
        )
        database.execute(
            SQL["removal_creator_assert"],
            (action_plan.operation_id, action_plan.creator_id),
        )
        consume_grant(database, action_plan.operation_id, action_plan.grant)
        database.execute(
            SQL["member_alias_remove_previous"],
            (action_plan.operation_id, NOW_MS),
        )
        database.execute(
            SQL["aliases_previous_changes_assert"],
            (action_plan.operation_id,),
        )
        database.execute(
            SQL["removal_delete"],
            (action_plan.operation_id, space_id),
        )
        database.execute(SQL["removal_changes_assert"], (action_plan.operation_id,))
        database.execute(
            SQL["removal_postcondition_assert"],
            (action_plan.operation_id, space_id),
        )
        authority_side_effects(
            database,
            action_plan,
            subject_query="authority_subject_insert_removed",
        )
        if single:
            database.execute(
                SQL["receipt_insert_remove_member"],
                (
                    action_plan.operation_id,
                    space_id,
                    action_plan.creator_id,
                    snapshot["actor_authority"],
                    NOW_MS,
                ),
            )
        else:
            database.execute(
                SQL["receipt_insert_batch_remove"],
                (
                    action_plan.operation_id,
                    space_id,
                    action_plan.creator_id,
                    snapshot["actor_authority"],
                    len(member_ids),
                    NOW_MS,
                ),
            )
        change_assert(database, action_plan.operation_id, "receipt_inserted", 1)
        insert_effect(database, action_plan)
        finish(database, action_plan)

    transaction(database, body)


def apply_set(
    database: sqlite3.Connection,
    action_plan: Plan,
    *,
    members: list[tuple[str, str]],
) -> None:
    if action_plan.space_id is None or action_plan.creator_id is None:
        raise AssertionError("set plan has no space/creator")
    snapshot = authority_snapshot(
        database,
        actor_id=action_plan.actor_id,
        organization_id=action_plan.organization_id,
        space_id=action_plan.space_id,
    )

    def body() -> None:
        claim(database, action_plan, snapshot)
        database.execute(
            SQL["final_members_insert"],
            (
                action_plan.operation_id,
                final_members_payload(action_plan, members),
            ),
        )
        creator_legacy_member_id, creator_mapped_member_id = generated_member_alias(
            action_plan, action_plan.creator_id, 500
        )
        database.execute(
            SQL["final_creator_upsert"],
            (
                action_plan.operation_id,
                action_plan.creator_id,
                action_plan.space_id,
                creator_legacy_member_id,
                creator_mapped_member_id,
            ),
        )
        graph_assertions(database, action_plan)
        database.execute(
            SQL["previous_snapshot_insert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        database.execute(
            SQL["previous_bound_assert"], (action_plan.operation_id,)
        )
        database.execute(
            SQL["previous_aliases_complete_assert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        consume_grant(database, action_plan.operation_id, action_plan.grant)
        database.execute(
            SQL["member_alias_remove_previous"],
            (action_plan.operation_id, NOW_MS),
        )
        database.execute(
            SQL["aliases_previous_changes_assert"],
            (action_plan.operation_id,),
        )
        database.execute(SQL["set_delete"], (action_plan.space_id,))
        database.execute(
            SQL["previous_changes_assert"], (action_plan.operation_id,)
        )
        database.execute(
            SQL["set_insert"],
            (action_plan.operation_id, action_plan.space_id, NOW_MS),
        )
        database.execute(
            SQL["final_changes_assert"], (action_plan.operation_id,)
        )
        database.execute(
            SQL["member_alias_insert_all"],
            (action_plan.operation_id, action_plan.space_id, NOW_MS),
        )
        database.execute(
            SQL["aliases_final_changes_assert"],
            (action_plan.operation_id,),
        )
        database.execute(
            SQL["member_alias_postcondition_assert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        database.execute(
            SQL["set_postcondition_assert"],
            (
                action_plan.operation_id,
                action_plan.space_id,
                action_plan.creator_id,
            ),
        )
        database.execute(
            SQL["out_of_scope_assert"],
            (action_plan.operation_id, action_plan.space_id),
        )
        authority_side_effects(database, action_plan)
        database.execute(
            SQL["receipt_insert_set"],
            (
                action_plan.operation_id,
                action_plan.space_id,
                action_plan.creator_id,
                snapshot["actor_authority"],
                NOW_MS,
            ),
        )
        change_assert(database, action_plan.operation_id, "receipt_inserted", 1)
        insert_effect(database, action_plan)
        finish(database, action_plan)

    transaction(database, body)


def operation_by_key(
    database: sqlite3.Connection,
    *,
    organization_id: str,
    actor_id: str,
    action: str,
    key_digest: str,
) -> sqlite3.Row | None:
    rows = database.execute(
        SQL["operation_by_key"],
        (organization_id, actor_id, action, key_digest),
    ).fetchall()
    if len(rows) > 1:
        raise AssertionError("operation lookup exceeded its uniqueness bound")
    return rows[0] if rows else None


def replay_existing(
    database: sqlite3.Connection,
    stored: Plan,
    replay_grant: Grant,
) -> None:
    snapshot = authority_snapshot(
        database,
        actor_id=stored.actor_id,
        organization_id=stored.organization_id,
        space_id=stored.space_id,
    )

    def body() -> None:
        assert_authority(
            database,
            stored.operation_id,
            stored.actor_id,
            stored.organization_id,
            snapshot,
        )
        if stored.space_id is not None:
            if stored.action in (BATCH_REMOVE_ACTION, REMOVE_MEMBER_ACTION):
                database.execute(
                    SQL["removed_target_graph_assert"],
                    (stored.operation_id, stored.organization_id),
                )
                database.execute(
                    SQL["creator_graph_assert"],
                    (
                        stored.operation_id,
                        stored.organization_id,
                        stored.creator_id,
                    ),
                )
            else:
                graph_assertions(database, stored)
        assert_grant(database, stored.operation_id, replay_grant)
        consume_grant(database, stored.operation_id, replay_grant)
        database.execute(
            SQL["proof_insert"],
            (
                replay_grant.grant_id,
                replay_grant.session_id,
                replay_grant.user_id,
                stored.operation_id,
                stored.organization_id,
                stored.action,
                stored.request_digest,
                "replay",
                NOW_MS + 1,
            ),
        )
        change_assert(database, stored.operation_id, "proof_journaled", 1)
        database.execute(SQL["assertion_cleanup"], (stored.operation_id,))

    transaction(database, body)


def consume_attempt(
    database: sqlite3.Connection,
    *,
    serial: int,
    grant: Grant,
    actor_id: str,
    organization_id: str,
    action: str,
    request_digest: str,
    outcome: str,
    related_operation_id: str | None,
) -> None:
    assertion_id = fixture_uuid(80_000 + serial)

    def body() -> None:
        assert_grant(database, assertion_id, grant)
        consume_grant(database, assertion_id, grant)
        database.execute(
            SQL["proof_insert"],
            (
                grant.grant_id,
                grant.session_id,
                grant.user_id,
                related_operation_id,
                organization_id,
                action,
                request_digest,
                outcome,
                NOW_MS + serial,
            ),
        )
        change_assert(database, assertion_id, "proof_journaled", 1)
        database.execute(SQL["assertion_cleanup"], (assertion_id,))

    if grant.user_id != actor_id:
        raise AssertionError("attempt proof actor does not match the request actor")
    transaction(database, body)


def expect_integrity_error(
    database: sqlite3.Connection,
    callback: Callable[[], None],
    *,
    message: str,
) -> None:
    database.execute("SAVEPOINT expected_integrity_error")
    try:
        callback()
    except sqlite3.IntegrityError as error:
        database.execute("ROLLBACK TO expected_integrity_error")
        database.execute("RELEASE expected_integrity_error")
        if message not in str(error):
            raise AssertionError(
                f"expected marker {message!r}, received {error!r}"
            ) from error
        return
    database.execute("ROLLBACK TO expected_integrity_error")
    database.execute("RELEASE expected_integrity_error")
    raise AssertionError("invalid membership state was accepted")


def test_full_migration_chain_and_checked_surface() -> None:
    database = migrated_database(through=39)
    seed_fixture(database)
    before = database.execute(
        "SELECT space_id,user_id,role FROM space_members ORDER BY space_id,user_id"
    ).fetchall()
    migration = MIGRATIONS / "0040_legacy_membership_actions_expand.sql"
    database.executescript(migration.read_text(encoding="utf-8"))
    assert database.execute("PRAGMA foreign_key_check").fetchall() == []
    assert database.execute(
        "SELECT space_id,user_id,role FROM space_members ORDER BY space_id,user_id"
    ).fetchall() == before

    tables = {
        row[0]
        for row in database.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        )
    }
    assert {
        "legacy_membership_action_operations_v1",
        "legacy_membership_action_final_members_v1",
        "legacy_membership_action_previous_members_v1",
        "legacy_membership_authority_generations_v1",
        "legacy_membership_action_authority_subjects_v1",
        "legacy_membership_action_revoked_grants_v1",
        "legacy_membership_action_receipts_v1",
        "legacy_membership_action_effects_v1",
        "legacy_membership_action_audit_events_v1",
        "legacy_membership_action_proof_consumptions_v1",
        "legacy_membership_action_assertions_v1",
        "legacy_space_member_aliases_v1",
    } <= tables
    assert len(SQL) == 78
    assert {
        "bulk_add_duplicate_assert",
        "bulk_add_insert",
        "bulk_add_postcondition_assert",
        "member_alias_targets",
        "removal_targets_insert",
        "removal_creator_assert",
        "removal_delete",
        "removal_postcondition_assert",
        "tenant_authority_assert",
        "receipt_insert_bulk_add",
        "receipt_insert_batch_remove",
        "receipt_insert_remove_member",
    } <= SQL.keys()
    assert max(
        (int(value) for sql in SQL.values() for value in re.findall(r"\?(\d+)", sql)),
        default=0,
    ) <= 100
    assert "LIMIT 2" in SQL["operation_by_key"]
    assert "LIMIT 100001" in SQL["previous_snapshot_insert"]
    assert "RETURNING id AS mutation_grant_id" in SQL["browser_grant_delete_returning"]
    assert "role = 'manager'" in SQL["final_creator_upsert"]
    assert "UNION" in SQL["authority_subject_insert"]

    application = APPLICATION.read_text(encoding="utf-8")
    runtime = RUNTIME.read_text(encoding="utf-8")
    for marker in (
        "cap-v1-866dbe8fbbfd7887",
        "cap-v1-455046db3d6ef019",
        "cap-v1-b177854e2386c877",
        "cap-v1-38aff8e7221d0260",
        "cap-v1-135614e516c47bf4",
        "cap-v1-9fc80bdec80fb248",
        "MAX_LEGACY_DISCOVERED_SPACE_MEMBERS: usize = 100_000",
        "ReplaceMembershipSetAndForceCreatorAdmin",
    ):
        assert marker in application
    for query_name in SQL:
        assert f"legacy_membership_actions/{query_name}.sql" in runtime
    assert "D1LegacyMembershipAtomicPortV1" in runtime
    assert "LegacyMembershipAtomicPortV1 for D1LegacyMembershipAtomicPortV1" in runtime
    assert "MAX_FRESH_SET_BATCH_STATEMENTS: usize = 43" in runtime
    assert 43 + 7 == 50
    database.close()


def test_remove_invite_replay_conflict_and_one_use_proofs() -> None:
    database = migrated_database()
    seed_fixture(database)
    applied_grant = seed_grant(database, 1, OWNER)
    action_plan = plan(
        1,
        actor_id=OWNER,
        action=REMOVE_ACTION,
        grant=applied_grant,
        invite_id=INVITE,
    )
    apply_remove(database, action_plan)
    assert database.execute(
        "SELECT COUNT(*) FROM organization_invites WHERE id=?1", (INVITE,)
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM organization_invites WHERE id=?1", (FOREIGN_INVITE,)
    ).fetchone()[0] == 1
    receipt = one_row(
        database,
        "SELECT * FROM legacy_membership_action_receipts_v1 WHERE operation_id=?1",
        (action_plan.operation_id,),
    )
    assert (
        receipt["result_kind"],
        receipt["matching_before"],
        receipt["deleted_rows"],
        receipt["matching_after"],
        receipt["actor_authority"],
    ) == ("organization_invite_removed", 1, 1, 0, "organization_owner")
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_membership_action_authority_subjects_v1 "
        "WHERE operation_id=?1",
        (action_plan.operation_id,),
    ).fetchone()[0] == 0

    replay_grant = seed_grant(database, 2, OWNER)
    replay_existing(database, action_plan, replay_grant)
    conflict_grant = seed_grant(database, 3, OWNER)
    conflict_digest = sha256("different-remove-request")
    consume_attempt(
        database,
        serial=3,
        grant=conflict_grant,
        actor_id=OWNER,
        organization_id=ORGANIZATION,
        action=REMOVE_ACTION,
        request_digest=conflict_digest,
        outcome="conflict",
        related_operation_id=action_plan.operation_id,
    )
    outcomes = [
        row[0]
        for row in database.execute(
            "SELECT outcome FROM legacy_membership_action_proof_consumptions_v1 "
            "WHERE related_operation_id=?1 ORDER BY consumed_at_ms,mutation_grant_id",
            (action_plan.operation_id,),
        )
    ]
    assert sorted(outcomes) == ["applied", "conflict", "replay"]
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_membership_action_operations_v1"
    ).fetchone()[0] == 1
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id IN (?1,?2,?3)",
        (applied_grant.grant_id, replay_grant.grant_id, conflict_grant.grant_id),
    ).fetchone()[0] == 0
    database.close()


def test_space_authority_classes_are_exact_and_non_broadening() -> None:
    database = migrated_database()
    seed_fixture(database)
    expected = {
        OWNER: "organization_owner",
        ADMIN: "organization_admin",
        CREATOR: "space_creator",
        MANAGER: "space_manager",
    }
    for actor_id, authority in expected.items():
        row = authority_snapshot(
            database,
            actor_id=actor_id,
            organization_id=ORGANIZATION,
            space_id=SPACE,
        )
        assert row["actor_authority"] == authority
    assert database.execute(
        SQL["space_authority_snapshot"], (ORDINARY, ORGANIZATION, SPACE)
    ).fetchall() == []
    database.execute(
        "UPDATE space_members SET role='contributor' WHERE space_id=?1 AND user_id=?2",
        (SPACE, MANAGER),
    )
    assert database.execute(
        SQL["space_authority_snapshot"], (MANAGER, ORGANIZATION, SPACE)
    ).fetchall() == []
    assert database.execute(
        SQL["space_authority_snapshot"], (OWNER, ORGANIZATION, FOREIGN_SPACE)
    ).fetchall() == []
    database.close()


def test_add_member_exact_role_generation_revocation_and_isolation() -> None:
    database = migrated_database()
    seed_fixture(database)
    browser_grant = seed_grant(database, 10, MANAGER)
    target_grant = seed_grant(database, 11, TARGET)
    action_plan = plan(
        10,
        actor_id=MANAGER,
        action=ADD_ACTION,
        grant=browser_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    before_other_space = database.execute(
        "SELECT user_id,role FROM space_members WHERE space_id=?1 ORDER BY user_id",
        (SPACE_TWO,),
    ).fetchall()
    apply_add(database, action_plan, target_id=TARGET, role="viewer")
    assert tuple(
        database.execute(
            "SELECT role,state,last_operation_id FROM space_members "
            "WHERE space_id=?1 AND user_id=?2",
            (SPACE, TARGET),
        ).fetchone()
    ) == ("viewer", "active", action_plan.operation_id)
    assert database.execute(
        "SELECT user_id,role FROM space_members WHERE space_id=?1 ORDER BY user_id",
        (SPACE_TWO,),
    ).fetchall() == before_other_space
    assert [
        tuple(row)
        for row in database.execute(
            "SELECT user_id FROM legacy_membership_action_authority_subjects_v1 "
            "WHERE operation_id=?1",
            (action_plan.operation_id,),
        )
    ] == [(TARGET,)]
    generation = one_row(
        database,
        "SELECT generation,last_operation_id FROM legacy_membership_authority_generations_v1 "
        "WHERE organization_id=?1 AND user_id=?2",
        (ORGANIZATION, TARGET),
    )
    assert tuple(generation) == (1, action_plan.operation_id)
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id IN (?1,?2)",
        (browser_grant.grant_id, target_grant.grant_id),
    ).fetchone()[0] == 0
    revoked = database.execute(
        "SELECT mutation_grant_id,user_id FROM legacy_membership_action_revoked_grants_v1 "
        "WHERE operation_id=?1",
        (action_plan.operation_id,),
    ).fetchall()
    assert [tuple(row) for row in revoked] == [(target_grant.grant_id, TARGET)]
    database.close()


def test_set_members_forces_creator_and_invalidates_exact_union() -> None:
    database = migrated_database()
    seed_fixture(database)
    browser_grant = seed_grant(database, 20, CREATOR)
    affected_grants = {
        CREATOR: seed_grant(database, 21, CREATOR),
        MANAGER: seed_grant(database, 22, MANAGER),
        OLD_MEMBER: seed_grant(database, 23, OLD_MEMBER),
        TARGET: seed_grant(database, 24, TARGET),
    }
    action_plan = plan(
        20,
        actor_id=CREATOR,
        action=SET_ACTION,
        grant=browser_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    before_other_space = database.execute(
        "SELECT user_id,role FROM space_members WHERE space_id=?1 ORDER BY user_id",
        (SPACE_TWO,),
    ).fetchall()
    apply_set(
        database,
        action_plan,
        members=[(CREATOR, "viewer"), (TARGET, "viewer")],
    )
    final_rows = [
        tuple(row)
        for row in database.execute(
            "SELECT user_id,role,state,last_operation_id FROM space_members "
            "WHERE space_id=?1 ORDER BY user_id",
            (SPACE,),
        )
    ]
    assert final_rows == [
        (CREATOR, "manager", "active", action_plan.operation_id),
        (TARGET, "viewer", "active", action_plan.operation_id),
    ]
    assert database.execute(
        "SELECT user_id,role FROM space_members WHERE space_id=?1 ORDER BY user_id",
        (SPACE_TWO,),
    ).fetchall() == before_other_space
    receipt = one_row(
        database,
        "SELECT matching_before,deleted_rows,inserted_rows,matching_after,result_count "
        "FROM legacy_membership_action_receipts_v1 WHERE operation_id=?1",
        (action_plan.operation_id,),
    )
    assert tuple(receipt) == (3, 3, 2, 2, 2)
    previous = {
        row[0]
        for row in database.execute(
            "SELECT user_id FROM legacy_membership_action_previous_members_v1 "
            "WHERE operation_id=?1",
            (action_plan.operation_id,),
        )
    }
    assert previous == {CREATOR, MANAGER, OLD_MEMBER}
    subjects = {
        row[0]
        for row in database.execute(
            "SELECT user_id FROM legacy_membership_action_authority_subjects_v1 "
            "WHERE operation_id=?1",
            (action_plan.operation_id,),
        )
    }
    assert subjects == {CREATOR, MANAGER, OLD_MEMBER, TARGET}
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_membership_authority_generations_v1 "
        "WHERE organization_id=?1 AND last_operation_id=?2",
        (ORGANIZATION, action_plan.operation_id),
    ).fetchone()[0] == 4
    all_grants = [browser_grant.grant_id, *[grant.grant_id for grant in affected_grants.values()]]
    placeholders = ",".join("?" for _ in all_grants)
    assert database.execute(
        f"SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id IN ({placeholders})",
        all_grants,
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_membership_action_revoked_grants_v1 "
        "WHERE operation_id=?1",
        (action_plan.operation_id,),
    ).fetchone()[0] == 4
    database.close()


def test_bulk_add_exact_mixed_noop_duplicate_and_alias_semantics() -> None:
    database = migrated_database()
    seed_fixture(database)

    mixed_grant = seed_grant(database, 25, MANAGER)
    mixed = plan(
        25,
        actor_id=MANAGER,
        action=ADD_MEMBERS_ACTION,
        grant=mixed_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    apply_bulk_add(
        database,
        mixed,
        members=[(OLD_MEMBER, "viewer"), (TARGET, "viewer")],
    )
    receipt = one_row(
        database,
        "SELECT result_kind,matching_before,inserted_rows,matching_after,result_count "
        "FROM legacy_membership_action_receipts_v1 WHERE operation_id=?1",
        (mixed.operation_id,),
    )
    assert tuple(receipt) == ("space_members_added", 3, 1, 4, 1)
    assert [
        row[0]
        for row in database.execute(
            "SELECT legacy_user_id FROM legacy_membership_action_previous_members_v1 "
            "WHERE operation_id=?1 ORDER BY user_id",
            (mixed.operation_id,),
        )
    ] == [
        USER_LEGACY_IDS[CREATOR],
        USER_LEGACY_IDS[MANAGER],
        USER_LEGACY_IDS[OLD_MEMBER],
    ]
    assert [
        tuple(row)
        for row in database.execute(
            "SELECT user_id FROM legacy_membership_action_authority_subjects_v1 "
            "WHERE operation_id=?1",
            (mixed.operation_id,),
        )
    ] == [(TARGET,)]
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_space_member_aliases_v1 "
        "WHERE space_id=?1 AND user_id=?2 AND removed_at_ms IS NULL",
        (SPACE, TARGET),
    ).fetchone()[0] == 1

    noop_grant = seed_grant(database, 26, MANAGER)
    noop = plan(
        26,
        actor_id=MANAGER,
        action=ADD_MEMBERS_ACTION,
        grant=noop_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    apply_bulk_add(
        database,
        noop,
        members=[(OLD_MEMBER, "viewer"), (OLD_MEMBER, "viewer")],
    )
    noop_receipt = one_row(
        database,
        "SELECT matching_before,inserted_rows,matching_after,result_count "
        "FROM legacy_membership_action_receipts_v1 WHERE operation_id=?1",
        (noop.operation_id,),
    )
    assert tuple(noop_receipt) == (4, 0, 4, 0)
    effect = one_row(
        database,
        "SELECT invalidates_space_page,invalidates_space_members,"
        "bumps_authority_generation,authority_subject_count "
        "FROM legacy_membership_action_effects_v1 WHERE operation_id=?1",
        (noop.operation_id,),
    )
    assert tuple(effect) == (1, 1, 0, 0)

    duplicate_grant = seed_grant(database, 27, MANAGER)
    duplicate = plan(
        27,
        actor_id=MANAGER,
        action=ADD_MEMBERS_ACTION,
        grant=duplicate_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    try:
        apply_bulk_add(
            database,
            duplicate,
            members=[(TARGET_TWO, "viewer"), (TARGET_TWO, "viewer")],
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_membership_conflict_v1" in str(error)
    else:
        raise AssertionError("duplicate new bulk targets partially committed")
    assert operation_by_key(
        database,
        organization_id=ORGANIZATION,
        actor_id=MANAGER,
        action=ADD_MEMBERS_ACTION,
        key_digest=duplicate.key_digest,
    ) is None
    assert database.execute(
        "SELECT COUNT(*) FROM space_members WHERE space_id=?1 AND user_id=?2",
        (SPACE, TARGET_TWO),
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (duplicate_grant.grant_id,),
    ).fetchone()[0] == 1
    database.close()


def test_batch_remove_partial_unmatched_replay_and_alias_immutability() -> None:
    database = migrated_database()
    seed_fixture(database)
    old_alias = SEEDED_MEMBER_ALIASES[(SPACE, OLD_MEMBER)]
    unmatched = (legacy_id(9_999), fixture_uuid(59_999))
    grant = seed_grant(database, 28, MANAGER)
    action_plan = plan(
        28,
        actor_id=MANAGER,
        action=BATCH_REMOVE_ACTION,
        grant=grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    submitted = [old_alias, unmatched, old_alias]
    apply_member_removal(
        database,
        action_plan,
        member_ids=submitted,
        single=False,
    )
    receipt = one_row(
        database,
        "SELECT result_kind,matching_before,deleted_rows,matching_after,result_count "
        "FROM legacy_membership_action_receipts_v1 WHERE operation_id=?1",
        (action_plan.operation_id,),
    )
    assert tuple(receipt) == ("space_members_removed", 1, 1, 0, len(submitted))
    assert database.execute(
        "SELECT COUNT(*) FROM space_members WHERE space_id=?1 AND user_id=?2",
        (SPACE, OLD_MEMBER),
    ).fetchone()[0] == 0
    alias = one_row(
        database,
        "SELECT removed_at_ms FROM legacy_space_member_aliases_v1 "
        "WHERE mapped_member_id=?1",
        (old_alias[1],),
    )
    assert alias[0] == NOW_MS
    assert [
        tuple(row)
        for row in database.execute(
            "SELECT user_id FROM legacy_membership_action_authority_subjects_v1 "
            "WHERE operation_id=?1",
            (action_plan.operation_id,),
        )
    ] == [(OLD_MEMBER,)]

    replay_grant = seed_grant(database, 29, MANAGER)
    replay_existing(database, action_plan, replay_grant)
    assert sorted(
        row[0]
        for row in database.execute(
            "SELECT outcome FROM legacy_membership_action_proof_consumptions_v1 "
            "WHERE related_operation_id=?1",
            (action_plan.operation_id,),
        )
    ) == ["applied", "replay"]
    expect_integrity_error(
        database,
        lambda: database.execute(
            "UPDATE legacy_space_member_aliases_v1 SET removed_at_ms=?2 "
            "WHERE mapped_member_id=?1",
            (old_alias[1], NOW_MS + 1),
        ),
        message="frame_legacy_membership_alias_immutable_v1",
    )
    expect_integrity_error(
        database,
        lambda: database.execute(
            "DELETE FROM legacy_space_member_aliases_v1 WHERE mapped_member_id=?1",
            (old_alias[1],),
        ),
        message="frame_legacy_membership_alias_immutable_v1",
    )
    database.close()


def test_remove_member_noop_missing_and_creator_constraints() -> None:
    database = migrated_database()
    seed_fixture(database)
    unmatched = (legacy_id(9_998), fixture_uuid(59_998))

    noop_grant = seed_grant(database, 33, ORDINARY)
    noop = plan(
        33,
        actor_id=ORDINARY,
        action=BATCH_REMOVE_ACTION,
        grant=noop_grant,
    )
    apply_member_removal(
        database,
        noop,
        member_ids=[unmatched, unmatched],
        single=False,
    )
    noop_receipt = one_row(
        database,
        "SELECT result_kind,space_id,actor_authority,matching_before,deleted_rows,"
        "matching_after,result_count FROM legacy_membership_action_receipts_v1 "
        "WHERE operation_id=?1",
        (noop.operation_id,),
    )
    assert tuple(noop_receipt) == (
        "space_members_removed",
        None,
        "active_organization_member",
        0,
        0,
        0,
        0,
    )

    old_alias = SEEDED_MEMBER_ALIASES[(SPACE, OLD_MEMBER)]
    single_grant = seed_grant(database, 34, MANAGER)
    single = plan(
        34,
        actor_id=MANAGER,
        action=REMOVE_MEMBER_ACTION,
        grant=single_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    apply_member_removal(
        database,
        single,
        member_ids=[old_alias],
        single=True,
    )
    assert one_row(
        database,
        "SELECT result_kind,matching_before,deleted_rows,matching_after "
        "FROM legacy_membership_action_receipts_v1 WHERE operation_id=?1",
        (single.operation_id,),
    )[:] == ("space_member_removed", 1, 1, 0)

    missing_grant = seed_grant(database, 35, MANAGER)
    missing = plan(
        35,
        actor_id=MANAGER,
        action=REMOVE_MEMBER_ACTION,
        grant=missing_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    try:
        apply_member_removal(
            database,
            missing,
            member_ids=[unmatched],
            single=True,
        )
    except LookupError:
        pass
    else:
        raise AssertionError("missing single removal returned success")
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (missing_grant.grant_id,),
    ).fetchone()[0] == 1

    creator_alias = SEEDED_MEMBER_ALIASES[(SPACE, CREATOR)]
    creator_grant = seed_grant(database, 36, MANAGER)
    creator_plan = plan(
        36,
        actor_id=MANAGER,
        action=REMOVE_MEMBER_ACTION,
        grant=creator_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    try:
        apply_member_removal(
            database,
            creator_plan,
            member_ids=[creator_alias],
            single=True,
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_membership_target_v1" in str(error)
    else:
        raise AssertionError("space creator was removed")
    assert database.execute(
        "SELECT COUNT(*) FROM space_members WHERE space_id=?1 AND user_id=?2",
        (SPACE, CREATOR),
    ).fetchone()[0] == 1
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (creator_grant.grant_id,),
    ).fetchone()[0] == 1
    database.close()


def test_batch_remove_cross_space_discovery_is_atomic() -> None:
    database = migrated_database()
    seed_fixture(database)
    second_alias = (legacy_id(7_777), fixture_uuid(57_777))
    database.execute(
        "INSERT INTO space_members(space_id,user_id,role,created_at_ms,updated_at_ms,state,revision) "
        "VALUES (?1,?2,'viewer',1,1,'active',0)",
        (SPACE_TWO, TARGET_TWO),
    )
    database.execute(
        "INSERT INTO legacy_space_member_aliases_v1(mapped_member_id,legacy_member_id,"
        "legacy_user_id,space_id,user_id,created_at_ms) VALUES (?1,?2,?3,?4,?5,1)",
        (second_alias[1], second_alias[0], USER_LEGACY_IDS[TARGET_TWO], SPACE_TWO, TARGET_TWO),
    )
    first_alias = SEEDED_MEMBER_ALIASES[(SPACE, OLD_MEMBER)]
    grant = seed_grant(database, 37, MANAGER)
    action_plan = plan(
        37,
        actor_id=MANAGER,
        action=BATCH_REMOVE_ACTION,
        grant=grant,
    )
    try:
        apply_member_removal(
            database,
            action_plan,
            member_ids=[first_alias, second_alias],
            single=False,
        )
    except RuntimeError as error:
        assert "cross-space" in str(error)
    else:
        raise AssertionError("cross-space batch removal was accepted")
    assert database.execute(
        "SELECT COUNT(*) FROM space_members WHERE (space_id=?1 AND user_id=?2) "
        "OR (space_id=?3 AND user_id=?4)",
        (SPACE, OLD_MEMBER, SPACE_TWO, TARGET_TWO),
    ).fetchone()[0] == 2
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (grant.grant_id,),
    ).fetchone()[0] == 1
    assert operation_by_key(
        database,
        organization_id=ORGANIZATION,
        actor_id=MANAGER,
        action=BATCH_REMOVE_ACTION,
        key_digest=action_plan.key_digest,
    ) is None
    database.close()


def test_targets_conflicts_and_cross_tenant_fail_without_partial_writes() -> None:
    database = migrated_database()
    seed_fixture(database)

    outsider_grant = seed_grant(database, 30, MANAGER)
    outsider_plan = plan(
        30,
        actor_id=MANAGER,
        action=ADD_ACTION,
        grant=outsider_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    try:
        apply_add(database, outsider_plan, target_id=OUTSIDER, role="viewer")
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_membership_target_v1" in str(error)
    else:
        raise AssertionError("foreign-tenant target was accepted")
    assert operation_by_key(
        database,
        organization_id=ORGANIZATION,
        actor_id=MANAGER,
        action=ADD_ACTION,
        key_digest=outsider_plan.key_digest,
    ) is None
    assert database.execute(
        "SELECT COUNT(*) FROM space_members WHERE space_id=?1 AND user_id=?2",
        (SPACE, OUTSIDER),
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (outsider_grant.grant_id,),
    ).fetchone()[0] == 1
    consume_attempt(
        database,
        serial=30,
        grant=outsider_grant,
        actor_id=MANAGER,
        organization_id=ORGANIZATION,
        action=ADD_ACTION,
        request_digest=outsider_plan.request_digest,
        outcome="rejected",
        related_operation_id=None,
    )

    duplicate_grant = seed_grant(database, 31, MANAGER)
    duplicate_plan = plan(
        31,
        actor_id=MANAGER,
        action=ADD_ACTION,
        grant=duplicate_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    try:
        apply_add(database, duplicate_plan, target_id=CREATOR, role="manager")
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_membership_conflict_v1" in str(error)
    else:
        raise AssertionError("duplicate space membership was accepted")
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (duplicate_grant.grant_id,),
    ).fetchone()[0] == 1

    foreign_invite_grant = seed_grant(database, 32, OWNER)
    foreign_invite_plan = plan(
        32,
        actor_id=OWNER,
        action=REMOVE_ACTION,
        grant=foreign_invite_grant,
        invite_id=FOREIGN_INVITE,
    )
    try:
        apply_remove(database, foreign_invite_plan)
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_membership_target_v1" in str(error)
    else:
        raise AssertionError("foreign-tenant invite was accepted")
    assert database.execute(
        "SELECT COUNT(*) FROM organization_invites WHERE id=?1",
        (FOREIGN_INVITE,),
    ).fetchone()[0] == 1
    assert database.execute(
        SQL["space_authority_snapshot"], (ORDINARY, ORGANIZATION, SPACE)
    ).fetchall() == []
    database.close()


def test_discovered_member_bound_aborts_before_delete() -> None:
    database = migrated_database()
    seed_fixture(database)
    database.execute("BEGIN")
    try:
        database.execute(
            """WITH RECURSIVE sequence(value) AS (
                 VALUES(1) UNION ALL SELECT value + 1 FROM sequence WHERE value < 100001
               )
               INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms,status)
               SELECT printf('10000000-0000-7000-8000-%012x',value),
                      printf('bulk-%d@example.invalid',value),
                      'bulk',1,1,'active'
               FROM sequence"""
        )
        database.execute(
            """INSERT INTO space_members(
                 space_id,user_id,role,created_at_ms,updated_at_ms,state,revision
               )
               SELECT ?1,id,'viewer',1,1,'active',0
               FROM users WHERE id LIKE '10000000-0000-7000-8000-%'""",
            (SPACE_TWO,),
        )
        database.execute(
            """INSERT INTO legacy_space_member_aliases_v1(
                 mapped_member_id,legacy_member_id,legacy_user_id,
                 space_id,user_id,created_at_ms
               )
               SELECT '20000000' || substr(id,9),
                      '9' || substr(replace(id,'-',''),-14),
                      '8' || substr(replace(id,'-',''),-14),
                      ?1,id,1
               FROM users WHERE id LIKE '10000000-0000-7000-8000-%'""",
            (SPACE_TWO,),
        )
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")
    before = database.execute(
        "SELECT COUNT(*) FROM space_members WHERE space_id=?1", (SPACE_TWO,)
    ).fetchone()[0]
    assert before == 100002
    grant = seed_grant(database, 40, CREATOR)
    action_plan = plan(
        40,
        actor_id=CREATOR,
        action=SET_ACTION,
        grant=grant,
        space_id=SPACE_TWO,
        creator_id=CREATOR,
    )
    try:
        apply_set(database, action_plan, members=[(TARGET, "viewer")])
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_membership_corrupt_v1" in str(error)
    else:
        raise AssertionError("100001st discovered member did not fail closed")
    assert database.execute(
        "SELECT COUNT(*) FROM space_members WHERE space_id=?1", (SPACE_TWO,)
    ).fetchone()[0] == before
    assert operation_by_key(
        database,
        organization_id=ORGANIZATION,
        actor_id=CREATOR,
        action=SET_ACTION,
        key_digest=action_plan.key_digest,
    ) is None
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (grant.grant_id,),
    ).fetchone()[0] == 1
    database.close()


def test_fault_rollback_and_completed_evidence_is_immutable() -> None:
    database = migrated_database()
    seed_fixture(database)
    grant = seed_grant(database, 50, MANAGER)
    failed_plan = plan(
        50,
        actor_id=MANAGER,
        action=ADD_ACTION,
        grant=grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    snapshot = authority_snapshot(
        database,
        actor_id=MANAGER,
        organization_id=ORGANIZATION,
        space_id=SPACE,
    )

    def injected_fault() -> None:
        claim(database, failed_plan, snapshot)
        database.execute(
            SQL["final_members_insert"],
            (
                failed_plan.operation_id,
                final_members_payload(failed_plan, [(TARGET, "viewer")]),
            ),
        )
        graph_assertions(database, failed_plan)
        database.execute(
            SQL["add_absent_assert"],
            (failed_plan.operation_id, SPACE, TARGET),
        )
        consume_grant(database, failed_plan.operation_id, grant)
        database.execute(
            SQL["add_insert"], (failed_plan.operation_id, SPACE, NOW_MS)
        )
        change_assert(database, failed_plan.operation_id, "members_inserted", 2)

    try:
        transaction(database, injected_fault)
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_membership_conflict_v1" in str(error)
    else:
        raise AssertionError("injected post-mutation mismatch committed")
    assert database.execute(
        "SELECT COUNT(*) FROM space_members WHERE space_id=?1 AND user_id=?2",
        (SPACE, TARGET),
    ).fetchone()[0] == 0
    assert database.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id=?1",
        (grant.grant_id,),
    ).fetchone()[0] == 1
    assert operation_by_key(
        database,
        organization_id=ORGANIZATION,
        actor_id=MANAGER,
        action=ADD_ACTION,
        key_digest=failed_plan.key_digest,
    ) is None

    success_grant = seed_grant(database, 51, MANAGER)
    success_plan = plan(
        51,
        actor_id=MANAGER,
        action=ADD_ACTION,
        grant=success_grant,
        space_id=SPACE,
        creator_id=CREATOR,
    )
    apply_add(database, success_plan, target_id=TARGET, role="viewer")
    expect_integrity_error(
        database,
        lambda: database.execute(
            "UPDATE legacy_membership_action_receipts_v1 SET matching_after=0 "
            "WHERE operation_id=?1",
            (success_plan.operation_id,),
        ),
        message="frame_legacy_membership_receipt_immutable_v1",
    )
    expect_integrity_error(
        database,
        lambda: database.execute(
            "DELETE FROM legacy_membership_action_final_members_v1 WHERE operation_id=?1",
            (success_plan.operation_id,),
        ),
        message="frame_legacy_membership_receipt_immutable_v1",
    )
    expect_integrity_error(
        database,
        lambda: database.execute(
            "UPDATE legacy_membership_action_proof_consumptions_v1 SET outcome='replay' "
            "WHERE mutation_grant_id=?1",
            (success_grant.grant_id,),
        ),
        message="frame_legacy_membership_proof_immutable_v1",
    )
    database.close()


def test_same_key_race_has_one_apply_and_one_replay() -> None:
    with tempfile.TemporaryDirectory(prefix="frame-membership-race-") as directory:
        path = Path(directory) / "race.sqlite"
        setup = migrated_database(path=path)
        seed_fixture(setup)
        first_grant = seed_grant(setup, 60, MANAGER)
        second_grant = seed_grant(setup, 61, MANAGER)
        shared_key = sha256("race-key")
        shared_request = sha256("race-request")
        first = plan(
            60,
            actor_id=MANAGER,
            action=ADD_ACTION,
            grant=first_grant,
            space_id=SPACE,
            creator_id=CREATOR,
            key_digest=shared_key,
            request_digest=shared_request,
        )
        second = plan(
            61,
            actor_id=MANAGER,
            action=ADD_ACTION,
            grant=second_grant,
            space_id=SPACE,
            creator_id=CREATOR,
            key_digest=shared_key,
            request_digest=shared_request,
        )
        setup.close()

        barrier = threading.Barrier(2)
        lock = threading.Lock()
        outcomes: list[str] = []
        failures: list[BaseException] = []

        def contender(action_plan: Plan) -> None:
            database = connect(path)
            try:
                assert operation_by_key(
                    database,
                    organization_id=ORGANIZATION,
                    actor_id=MANAGER,
                    action=ADD_ACTION,
                    key_digest=shared_key,
                ) is None
                barrier.wait(timeout=20)
                try:
                    apply_add(database, action_plan, target_id=TARGET, role="viewer")
                    outcome = "applied"
                except sqlite3.IntegrityError:
                    row = operation_by_key(
                        database,
                        organization_id=ORGANIZATION,
                        actor_id=MANAGER,
                        action=ADD_ACTION,
                        key_digest=shared_key,
                    )
                    if row is None or row["request_digest"] != shared_request:
                        raise
                    winner = first if row["operation_id"] == first.operation_id else second
                    replay_existing(database, winner, action_plan.grant)
                    outcome = "replay"
                with lock:
                    outcomes.append(outcome)
            except BaseException as error:  # noqa: BLE001 - thread handoff
                with lock:
                    failures.append(error)
            finally:
                database.close()

        threads = [
            threading.Thread(target=contender, args=(first,)),
            threading.Thread(target=contender, args=(second,)),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=30)
        if failures:
            raise failures[0]
        assert sorted(outcomes) == ["applied", "replay"]
        verify = connect(path)
        assert verify.execute(
            "SELECT COUNT(*) FROM legacy_membership_action_operations_v1 "
            "WHERE organization_id=?1 AND actor_id=?2 AND action=?3",
            (ORGANIZATION, MANAGER, ADD_ACTION),
        ).fetchone()[0] == 1
        assert verify.execute(
            "SELECT COUNT(*) FROM space_members WHERE space_id=?1 AND user_id=?2",
            (SPACE, TARGET),
        ).fetchone()[0] == 1
        assert sorted(
            row[0]
            for row in verify.execute(
                "SELECT outcome FROM legacy_membership_action_proof_consumptions_v1 "
                "WHERE action=?1",
                (ADD_ACTION,),
            )
        ) == ["applied", "replay"]
        verify.close()


def artifact_digest(paths: list[Path]) -> str:
    digest = hashlib.sha256()
    for path in sorted(paths):
        relative = path.relative_to(ROOT).as_posix().encode()
        content = path.read_bytes()
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(len(content).to_bytes(8, "big"))
        digest.update(content)
    return digest.hexdigest()


TESTS: tuple[tuple[str, Callable[[], None]], ...] = (
    ("full_migration_chain_and_checked_surface", test_full_migration_chain_and_checked_surface),
    (
        "remove_invite_replay_conflict_and_one_use_proofs",
        test_remove_invite_replay_conflict_and_one_use_proofs,
    ),
    ("space_authority_classes_are_exact", test_space_authority_classes_are_exact_and_non_broadening),
    (
        "add_member_generation_revocation_and_isolation",
        test_add_member_exact_role_generation_revocation_and_isolation,
    ),
    (
        "set_members_creator_and_exact_union",
        test_set_members_forces_creator_and_invalidates_exact_union,
    ),
    (
        "bulk_add_mixed_noop_duplicate_aliases",
        test_bulk_add_exact_mixed_noop_duplicate_and_alias_semantics,
    ),
    (
        "batch_remove_partial_replay_alias_immutability",
        test_batch_remove_partial_unmatched_replay_and_alias_immutability,
    ),
    (
        "single_remove_noop_missing_creator_constraints",
        test_remove_member_noop_missing_and_creator_constraints,
    ),
    (
        "batch_remove_cross_space_atomicity",
        test_batch_remove_cross_space_discovery_is_atomic,
    ),
    (
        "targets_conflicts_and_cross_tenant_rollback",
        test_targets_conflicts_and_cross_tenant_fail_without_partial_writes,
    ),
    ("discovered_member_bound", test_discovered_member_bound_aborts_before_delete),
    ("fault_rollback_and_immutability", test_fault_rollback_and_completed_evidence_is_immutable),
    ("same_key_race_apply_replay", test_same_key_race_has_one_apply_and_one_replay),
)


def evidence(results: list[dict[str, str]]) -> dict[str, Any]:
    migrations = migration_paths()
    query_paths = sorted(QUERIES.glob("*.sql"))
    return {
        "schema": "frame.legacy-membership-actions-sqlite-conformance.v2",
        "status": "passed",
        "runtime_boundary": "python_sqlite3_full_expand_chain_and_checked_in_d1_queries",
        "test_count": len(results),
        "query_count": len(query_paths),
        "migration_count": len(migrations),
        "results": results,
        "query_files": [path.name for path in query_paths],
        "migration_files": [path.name for path in migrations],
        "digests": {
            "migrations_sha256": artifact_digest(migrations),
            "queries_sha256": artifact_digest(query_paths),
            "application_sha256": hashlib.sha256(APPLICATION.read_bytes()).hexdigest(),
            "runtime_sha256": hashlib.sha256(RUNTIME.read_bytes()).hexdigest(),
            "migration_0040_sha256": hashlib.sha256(
                (MIGRATIONS / "0040_legacy_membership_actions_expand.sql").read_bytes()
            ).hexdigest(),
        },
        "invariants": [
            "active_tenant_authority_is_reasserted_in_the_committing_transaction",
            "foreign_or_inactive_targets_fail_without_scope_disclosure",
            "creator_is_forced_to_manager_and_contributor_is_never_written",
            "bulk_add_preserves_mixed_and_all_already_semantics_and_rejects_duplicate_new_targets",
            "batch_remove_deletes_only_resolved_single_space_non_creator_aliases",
            "all_unmatched_batch_remove_is_an_active_tenant_member_noop",
            "single_remove_requires_one_live_alias_and_creator_is_immutable",
            "prior_and_final_subject_union_is_generation_bumped_and_grant_revoked",
            "same_key_same_fingerprint_replays_without_reapplying",
            "same_key_different_fingerprint_conflicts_and_consumes_one_proof",
            "the_100001st_discovered_member_aborts_before_delete",
            "receipt_effect_audit_proof_and_operation_commit_atomically",
            "worst_case_fresh_set_uses_50_d1_queries_and_at_most_100_bindings_per_statement",
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path)
    arguments = parser.parse_args()
    results: list[dict[str, str]] = []
    for name, test in TESTS:
        test()
        results.append({"name": name, "status": "passed"})
    payload = evidence(results)
    if arguments.evidence is not None:
        arguments.evidence.parent.mkdir(parents=True, exist_ok=True)
        arguments.evidence.write_text(
            json.dumps(payload, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    print(
        "legacy membership action SQLite conformance: "
        f"{len(results)} passed; {len(SQL)} bounded queries; full migration chain, "
        "authority, creator forcing, replay/conflict, proof consumption, rollback, "
        "generation/revocation, race, and immutability verified"
    )


if __name__ == "__main__":
    main()
