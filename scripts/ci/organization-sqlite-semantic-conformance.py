#!/usr/bin/env python3
"""Offline semantic checks for the repaired organization authority workflows.

This suite executes the checked-in migrations and SQL with Python's sqlite3.
It deliberately does not claim Worker/Wasm, Wrangler, or D1-provider parity.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import pathlib
import re
import sqlite3
import sys
import time
from collections.abc import Callable, Sequence
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
CONTROL = ROOT / "apps" / "control-plane"
MIGRATIONS = CONTROL / "migrations"
QUERIES = CONTROL / "queries" / "organization"
REPOSITORY_SOURCE = CONTROL / "src" / "organization_repository.rs"
PORT_SOURCE = ROOT / "crates" / "ports" / "src" / "organization.rs"
PLACEHOLDER = re.compile(r"\?([1-9][0-9]*)")
CAS_SENTINEL = "frame_organization_cas_conflict_v1"
RETENTION_SENTINEL = "frame_organization_retention_locked_v1"
NOW_MS = int(time.time()) * 1_000

ORG_A = "018f47a6-7b1c-7f55-8f39-8f8a8690e301"
ORG_B = "018f47a6-7b1c-7f55-8f39-8f8a8690e302"
ORG_RETENTION = "018f47a6-7b1c-7f55-8f39-8f8a8690e303"
ORG_EXPIRED = "018f47a6-7b1c-7f55-8f39-8f8a8690e304"
ORG_DELETED = "018f47a6-7b1c-7f55-8f39-8f8a8690e305"
OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a301"
ADMIN = "018f47a6-7b1c-7f55-8f39-8f8a8690a302"
CONTRIBUTOR = "018f47a6-7b1c-7f55-8f39-8f8a8690a303"
VIEWER = "018f47a6-7b1c-7f55-8f39-8f8a8690a304"
INVITEE = "018f47a6-7b1c-7f55-8f39-8f8a8690a305"
OUTSIDER = "018f47a6-7b1c-7f55-8f39-8f8a8690a306"
SUPPORT = "018f47a6-7b1c-7f55-8f39-8f8a8690a307"
SPACE_A = "018f47a6-7b1c-7f55-8f39-8f8a8690b301"
SPACE_B = "018f47a6-7b1c-7f55-8f39-8f8a8690b302"
SPACE_CREATED = "018f47a6-7b1c-7f55-8f39-8f8a8690b303"
FOLDER_OWNED = "018f47a6-7b1c-7f55-8f39-8f8a8690c301"
FOLDER_OTHER = "018f47a6-7b1c-7f55-8f39-8f8a8690c302"
INVITE = "018f47a6-7b1c-7f55-8f39-8f8a8690f301"
INVITE_LIST = "018f47a6-7b1c-7f55-8f39-8f8a8690f302"
FIXTURE_OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a8690d301"
OP_INVITE = "018f47a6-7b1c-7f55-8f39-8f8a8690d302"
OP_REPLAY = "018f47a6-7b1c-7f55-8f39-8f8a8690d303"
OP_FOLDER = "018f47a6-7b1c-7f55-8f39-8f8a8690d304"
OP_TOMBSTONE = "018f47a6-7b1c-7f55-8f39-8f8a8690d305"
OP_RECOVER = "018f47a6-7b1c-7f55-8f39-8f8a8690d306"
OP_SUPPORT = "018f47a6-7b1c-7f55-8f39-8f8a8690d307"
OP_SPACE = "018f47a6-7b1c-7f55-8f39-8f8a8690d308"
OP_SHARE = "018f47a6-7b1c-7f55-8f39-8f8a8690d309"
OP_DOMAIN = "018f47a6-7b1c-7f55-8f39-8f8a8690d310"
OP_CREATE_SPACE = "018f47a6-7b1c-7f55-8f39-8f8a8690d311"
OP_PLAN = "018f47a6-7b1c-7f55-8f39-8f8a8690d312"
AUDIT_SUPPORT = "018f47a6-7b1c-7f55-8f39-8f8a8690aa01"
AUDIT_REPLAY = "018f47a6-7b1c-7f55-8f39-8f8a8690aa02"
AUDIT_PLAN = "018f47a6-7b1c-7f55-8f39-8f8a8690aa03"
PLAN_ID = "018f47a6-7b1c-7f55-8f39-8f8a8690ab01"
IDENTIFIER_DIGEST = f"{11:064x}"
TOKEN_DIGEST = f"{21:064x}"
OTHER_TOKEN_DIGEST = f"{22:064x}"
SUPPORT_DIGEST = f"{91:064x}"


class ConformanceFailure(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ConformanceFailure(message)


def migration_files() -> list[pathlib.Path]:
    files = sorted(MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql"))
    require(
        [int(path.name[:4]) for path in files] == list(range(1, len(files) + 1)),
        "migration sequence is not contiguous",
    )
    return files


def query(name: str) -> str:
    return (QUERIES / name).read_text(encoding="utf-8").strip()


def migrate(database: sqlite3.Connection) -> None:
    database.execute("PRAGMA foreign_keys = ON")
    for path in migration_files():
        database.executescript(path.read_text(encoding="utf-8"))


def compile_all_queries(database: sqlite3.Connection) -> None:
    files = sorted(QUERIES.glob("*.sql"))
    require(len(files) >= 60, "organization query inventory is incomplete")
    for path in files:
        sql = path.read_text(encoding="utf-8").strip()
        indexes = [int(match) for match in PLACEHOLDER.findall(sql)]
        require(bool(indexes), f"query has no numbered binding contract: {path.name}")
        database.execute("EXPLAIN " + sql, [None] * max(indexes)).fetchall()


def add_user(database: sqlite3.Connection, user_id: str, label: str) -> None:
    database.execute(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) "
        "VALUES (?,?,NULL,?,?)",
        (user_id, f"{label}@sqlite.invalid", NOW_MS - 20_000, NOW_MS - 20_000),
    )
    database.execute(
        "INSERT INTO auth_identities_v2(user_id,identity_revision,session_version,"
        "created_at_ms,updated_at_ms,revision,last_operation_id) VALUES (?,1,0,?,?,0,?)",
        (user_id, NOW_MS - 20_000, NOW_MS - 20_000, FIXTURE_OPERATION),
    )


def add_organization(
    database: sqlite3.Connection,
    organization_id: str,
    owner_id: str,
    name: str,
    *,
    status: str = "active",
    tombstoned_at_ms: int | None = None,
    retention_until_ms: int | None = None,
    revision: int = 0,
    authority_version: int = 0,
) -> None:
    database.execute(
        "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,"
        "updated_at_ms,tombstoned_at_ms,revision,authority_version,retention_until_ms,"
        "recovered_at_ms,last_operation_id) VALUES (?,?,?,?,'{}',?,?,?,?,?,?,NULL,?)",
        (
            organization_id,
            owner_id,
            name,
            status,
            NOW_MS - 10_000,
            NOW_MS - 10_000,
            tombstoned_at_ms,
            revision,
            authority_version,
            retention_until_ms,
            FIXTURE_OPERATION,
        ),
    )


def add_membership(
    database: sqlite3.Connection,
    organization_id: str,
    user_id: str,
    role: str,
) -> None:
    database.execute(
        "INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,"
        "created_at_ms,updated_at_ms,revision,authority_version,last_operation_id) "
        "VALUES (?,?,?,'active',0,?,?,0,0,?)",
        (organization_id, user_id, role, NOW_MS - 9_000, NOW_MS - 9_000, FIXTURE_OPERATION),
    )


def add_space(
    database: sqlite3.Connection,
    space_id: str,
    organization_id: str,
    creator_id: str,
    name: str,
) -> None:
    database.execute(
        "INSERT INTO spaces(id,organization_id,created_by_user_id,name,is_primary,is_public,"
        "settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision,authority_version,"
        "last_operation_id) VALUES (?,?,?, ?,1,0,'{}',?,?,NULL,0,0,?)",
        (space_id, organization_id, creator_id, name, NOW_MS - 8_000, NOW_MS - 8_000, FIXTURE_OPERATION),
    )


def seed(database: sqlite3.Connection) -> None:
    for user_id, label in (
        (OWNER, "owner"),
        (ADMIN, "admin"),
        (CONTRIBUTOR, "contributor"),
        (VIEWER, "viewer"),
        (INVITEE, "invitee"),
        (OUTSIDER, "outsider"),
        (SUPPORT, "support"),
    ):
        add_user(database, user_id, label)

    add_organization(database, ORG_A, OWNER, "Organization A")
    add_organization(database, ORG_B, OUTSIDER, "Organization B")
    add_organization(database, ORG_RETENTION, OWNER, "Retention")
    add_organization(
        database,
        ORG_EXPIRED,
        OWNER,
        "Expired retention",
        status="tombstoned",
        tombstoned_at_ms=NOW_MS - 20_000,
        retention_until_ms=NOW_MS - 10_000,
        revision=1,
        authority_version=1,
    )
    add_organization(database, ORG_DELETED, OWNER, "Deleted", status="deleted")
    for organization_id, user_id, role in (
        (ORG_A, OWNER, "owner"),
        (ORG_A, ADMIN, "admin"),
        (ORG_A, CONTRIBUTOR, "member"),
        (ORG_A, VIEWER, "viewer"),
        (ORG_B, OUTSIDER, "owner"),
        (ORG_RETENTION, OWNER, "owner"),
        (ORG_EXPIRED, OWNER, "owner"),
        (ORG_DELETED, OWNER, "owner"),
    ):
        add_membership(database, organization_id, user_id, role)

    add_space(database, SPACE_A, ORG_A, OWNER, "Space A")
    add_space(database, SPACE_B, ORG_B, OUTSIDER, "Space B")
    for user_id, role in ((CONTRIBUTOR, "contributor"), (VIEWER, "viewer")):
        database.execute(
            "INSERT INTO space_members(space_id,user_id,role,created_at_ms,updated_at_ms,"
            "state,revision,last_operation_id) VALUES (?,?,?, ?,?,'active',0,?)",
            (SPACE_A, user_id, role, NOW_MS - 7_000, NOW_MS - 7_000, FIXTURE_OPERATION),
        )
    for folder_id, creator_id, name in (
        (FOLDER_OWNED, CONTRIBUTOR, "Contributor folder"),
        (FOLDER_OTHER, OWNER, "Owner folder"),
    ):
        database.execute(
            "INSERT INTO folders(id,organization_id,space_id,parent_id,created_by_user_id,name,"
            "is_public,settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision,depth,"
            "tree_revision,last_operation_id) VALUES (?,?,?,NULL,?,?,0,'{}',?,?,NULL,0,0,0,?)",
            (
                folder_id,
                ORG_A,
                SPACE_A,
                creator_id,
                name,
                NOW_MS - 6_000,
                NOW_MS - 6_000,
                FIXTURE_OPERATION,
            ),
        )
        database.execute(
            "INSERT INTO organization_folder_closure_v1(organization_id,space_id,ancestor_id,"
            "descendant_id,distance) VALUES (?,?,?,?,0)",
            (ORG_A, SPACE_A, folder_id, folder_id),
        )

    database.execute(
        "INSERT INTO auth_identifier_digests_v2(key_version,digest,user_id,created_at_ms,"
        "last_operation_id) VALUES (1,?,?,?,?)",
        (IDENTIFIER_DIGEST, INVITEE, NOW_MS - 5_000, FIXTURE_OPERATION),
    )
    for invite_id, invited_digest, token_digest in (
        (INVITE, IDENTIFIER_DIGEST, TOKEN_DIGEST),
        (INVITE_LIST, f"{12:064x}", f"{23:064x}"),
    ):
        database.execute(
            "INSERT INTO organization_invites(id,organization_id,invited_email_digest,"
            "invited_email_key_version,invited_by_user_id,role,status,token_digest,created_at_ms,"
            "expires_at_ms,resolved_at_ms,revision,accepted_by_user_id,last_operation_id) "
            "VALUES (?,?,?,1,?,'member','pending',?,?,?,NULL,0,NULL,?)",
            (
                invite_id,
                ORG_A,
                invited_digest,
                ADMIN,
                token_digest,
                NOW_MS - 1_000,
                NOW_MS + 120_000,
                FIXTURE_OPERATION,
            ),
        )
    for domain in ("alpha.example", "beta.example"):
        database.execute(
            "INSERT INTO organization_allowed_domains(organization_id,domain_ascii,"
            "verified_at_ms,created_at_ms,revision,last_operation_id) VALUES (?,?,NULL,?,0,?)",
            (ORG_A, domain, NOW_MS - 4_000, FIXTURE_OPERATION),
        )
    database.execute(
        "INSERT INTO organization_support_authorities_v1(support_actor_id,organization_id,"
        "ticket_digest,issued_at_ms,expires_at_ms,revoked_at_ms) VALUES (?,?,?,?,?,NULL)",
        (SUPPORT, ORG_A, SUPPORT_DIGEST, NOW_MS - 1_000, NOW_MS + 120_000),
    )
    database.execute(
        "INSERT INTO organization_support_authorities_v1(support_actor_id,organization_id,"
        "ticket_digest,issued_at_ms,expires_at_ms,revoked_at_ms) VALUES (?,?,?,?,?,NULL)",
        (SUPPORT, ORG_DELETED, SUPPORT_DIGEST, NOW_MS - 1_000, NOW_MS + 120_000),
    )
    database.execute(
        "UPDATE users SET active_organization_id=?,organization_preference_revision=1 "
        "WHERE id=?",
        (ORG_A, OUTSIDER),
    )
    database.commit()


def expect_trigger(
    database: sqlite3.Connection,
    sentinel: str,
    action: Callable[[], None],
) -> None:
    database.execute("BEGIN IMMEDIATE")
    try:
        action()
    except sqlite3.IntegrityError as error:
        database.rollback()
        require(sentinel in str(error), "unexpected SQLite trigger classification")
    else:
        database.rollback()
        raise ConformanceFailure("expected transactional assertion failure")


def successful_batch(database: sqlite3.Connection, action: Callable[[], None]) -> None:
    database.execute("BEGIN IMMEDIATE")
    try:
        action()
    except Exception:
        database.rollback()
        raise
    database.commit()


def scenario_dirty_upgrade() -> dict[str, Any]:
    path = ROOT / "scripts" / "ci" / "test-migration.py"
    spec = importlib.util.spec_from_file_location("frame_dirty_organization_migration", path)
    require(spec is not None and spec.loader is not None, "dirty migration test could not load")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    module.exercise_dirty_organization_upgrade()
    return {
        "legacy_multi_owner_upgrade": "pass",
        "legacy_nested_and_cycle_graph": "pass",
        "reviewed_normalization_and_contract_index": "pass",
    }


def scenario_invite_identity(database: sqlite3.Connection) -> dict[str, Any]:
    def wrong_actor() -> None:
        database.execute(
            query("invite_accept_eligibility_assert.sql"),
            (
                f"{OP_INVITE}:wrong",
                INVITE,
                ORG_A,
                0,
                OUTSIDER,
                TOKEN_DIGEST,
                1,
                0,
            ),
        )

    expect_trigger(database, CAS_SENTINEL, wrong_actor)
    require(
        database.execute(
            "SELECT status,accepted_by_user_id FROM organization_invites WHERE id=?", (INVITE,)
        ).fetchone()
        == ("pending", None),
        "token holder changed an invite without matching authenticated identity",
    )

    def accept() -> None:
        database.execute(
            query("invite_accept_eligibility_assert.sql"),
            (
                f"{OP_INVITE}:eligibility",
                INVITE,
                ORG_A,
                0,
                INVITEE,
                TOKEN_DIGEST,
                1,
                0,
            ),
        )
        database.execute(
            query("invite_accept.sql"),
            (INVITE, ORG_A, 0, TOKEN_DIGEST, INVITEE, OP_INVITE),
        )
        database.execute(
            query("invite_accept_membership.sql"),
            (INVITE, INVITEE, NOW_MS, OP_INVITE),
        )
        database.execute(
            query("invite_membership_postcondition.sql"),
            (f"{OP_INVITE}:membership_post", INVITE, ORG_A, INVITEE, OP_INVITE),
        )
        database.execute(
            query("invite_postcondition.sql"),
            (f"{OP_INVITE}:post", INVITE, ORG_A, "accepted", 1, OP_INVITE),
        )
        database.execute(query("assertion_cleanup.sql"), (OP_INVITE,))

    successful_batch(database, accept)
    require(
        database.execute(
            "SELECT status,accepted_by_user_id,revision FROM organization_invites WHERE id=?",
            (INVITE,),
        ).fetchone()
        == ("accepted", INVITEE, 1),
        "identity-bound invite acceptance did not commit",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM organization_members WHERE organization_id=? AND user_id=? "
            "AND role='member' AND state='active'",
            (ORG_A, INVITEE),
        ).fetchone()
        == (1,),
        "invite acceptance did not create exactly one membership",
    )
    return {
        "wrong_actor_with_token": "denied_without_mutation",
        "authenticated_identifier_match": "accepted_once",
    }


def semantic_fingerprint(operation_kind: str, actor_id: str, payload: dict[str, Any]) -> str:
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()
    digest = hashlib.sha256()
    digest.update(b"frame/organization/semantic-request/v2\0")
    for part in (operation_kind.encode(), actor_id.encode(), encoded):
        digest.update(len(part).to_bytes(8, "big"))
        digest.update(part)
    return digest.hexdigest()


def scenario_semantic_replay(database: sqlite3.Connection) -> dict[str, Any]:
    payload = {
        "expected_invite_revision": 0,
        "invite_id": INVITE,
        "presented_token_digest": TOKEN_DIGEST,
    }
    changed_payload = dict(payload, presented_token_digest=OTHER_TOKEN_DIGEST)
    exact = semantic_fingerprint("invite_accept", INVITEE, payload)
    changed = semantic_fingerprint("invite_accept", INVITEE, changed_payload)
    require(
        exact == "8ebd0facc77e48b7cd952d998518355b8587d2d64c757f8f9cc1fe2ac89f3a9a",
        "semantic fingerprint canonical test vector drifted",
    )
    require(exact != changed, "semantic payload mismatch produced the same fingerprint")
    database.execute(
        query("operation_insert.sql"),
        (OP_REPLAY, ORG_A, "semantic-replay-0001", "invite_accept", INVITE, exact, "accepted", 1, 0, NOW_MS),
    )
    database.execute(
        query("audit_insert.sql"),
        (
            AUDIT_REPLAY,
            OP_REPLAY,
            ORG_A,
            INVITEE,
            "invite_accept",
            "invite",
            hashlib.sha256(INVITE.encode()).hexdigest(),
            "allow",
            None,
            NOW_MS,
        ),
    )
    database.commit()
    row = database.execute(
        query("operation_by_idempotency.sql"),
        (ORG_A, "semantic-replay-0001", INVITEE, 1, 0),
    ).fetchone()
    require(row is not None and row[5] == exact, "stored semantic replay fingerprint drifted")

    def classify_replay(candidate: tuple[Any, ...] | None, fingerprint: str) -> str:
        if candidate is None:
            return "absent"
        matches = (
            candidate[0] == OP_REPLAY
            and candidate[1] == ORG_A
            and candidate[2] == "semantic-replay-0001"
            and candidate[3] == "invite_accept"
            and candidate[4] == INVITE
            and candidate[5] == fingerprint
        )
        return "replay" if matches else "conflict"

    require(classify_replay(row, exact) == "replay", "exact replay classifier drifted")
    require(
        classify_replay(row, changed) == "conflict",
        "changed payload was accepted as an exact replay",
    )
    unauthorized_existing = database.execute(
        query("operation_by_idempotency.sql"),
        (ORG_A, "semantic-replay-0001", OUTSIDER, 1, 0),
    ).fetchone()
    unauthorized_absent = database.execute(
        query("operation_by_idempotency.sql"),
        (ORG_A, "semantic-replay-absent", OUTSIDER, 1, 0),
    ).fetchone()
    require(
        unauthorized_existing is None and unauthorized_absent is None,
        "receipt lookup disclosed existing versus absent state to another actor",
    )

    source = REPOSITORY_SOURCE.read_text(encoding="utf-8")
    ports = PORT_SOURCE.read_text(encoding="utf-8")
    context_start = ports.index("pub struct OrganizationMutationContext")
    context_end = ports.index("}", context_start)
    require(
        "request_fingerprint" not in ports[context_start:context_end],
        "caller-controlled fingerprint remains in the mutation context",
    )
    for marker in (
        "frame/organization/semantic-request/v2\\0",
        "semantic_fingerprint(\n            \"invite_accept\"",
        "request_fingerprint != request_fingerprint.expose_for_verification()",
    ):
        require(marker in source, "server-derived semantic replay binding drifted")
    return {
        "exact_replay": "matches_stored_receipt",
        "same_key_changed_payload": "conflict",
        "fingerprint_authority": "server_derived",
        "unauthorized_existing_vs_absent": "indistinguishable",
    }


def tenant_state_digest(database: sqlite3.Connection) -> str:
    digest = hashlib.sha256()
    for table in (
        "organizations",
        "organization_members",
        "organization_invites",
        "organization_allowed_domains",
        "spaces",
        "space_members",
        "folders",
        "organization_repository_operations_v1",
        "organization_audit_events_v1",
    ):
        rows = database.execute(f"SELECT * FROM {table} ORDER BY rowid").fetchall()
        digest.update(table.encode())
        digest.update(repr(rows).encode())
    return digest.hexdigest()


def scenario_denial_nonmutation(database: sqlite3.Connection) -> dict[str, Any]:
    before = tenant_state_digest(database)
    before_audits = database.execute(
        "SELECT COUNT(*) FROM organization_audit_events_v1"
    ).fetchone()[0]

    def denied_write() -> None:
        database.execute(
            query("operation_absent_assert.sql"),
            (
                f"{OP_FOLDER}:operation",
                OP_FOLDER,
                ORG_A,
                "denial-rollback-0001",
            ),
        )
        database.execute(
            query("authority_assert.sql"),
            (
                f"{OP_FOLDER}:denied",
                ORG_A,
                OUTSIDER,
                1,
                0,
                "active",
                0,
                0,
                0,
                0,
                "write",
            ),
        )

    expect_trigger(database, CAS_SENTINEL, denied_write)
    require(before == tenant_state_digest(database), "denial mutated tenant state")
    require(
        database.execute(
            "SELECT COUNT(*) FROM organization_repository_assertions_v1 "
            "WHERE id=?",
            (f"{OP_FOLDER}:operation",),
        ).fetchone()
        == (0,),
        "successful pre-authority assertion survived denial rollback",
    )
    require(
        before_audits
        == database.execute("SELECT COUNT(*) FROM organization_audit_events_v1").fetchone()[0],
        "denial wrote an attacker-selected tenant audit row",
    )
    source = REPOSITORY_SOURCE.read_text(encoding="utf-8")
    denial_start = source.index("async fn audit_decision_inner")
    denial_body = source[denial_start : source.index("\n    }\n}", denial_start)]
    require("self.statement(" not in denial_body, "external denial reporting writes tenant SQL")
    require(
        'OrganizationRepositoryTelemetry::emit(decision.action.stable_code(), outcome, 0);'
        in denial_body,
        "denial telemetry boundary drifted",
    )
    return {
        "cross_tenant_write": "denied",
        "tenant_rows_changed": 0,
        "tenant_audit_rows_added": 0,
    }


def assertion_succeeds(
    database: sqlite3.Connection, sql_name: str, bindings: Sequence[object]
) -> None:
    database.execute("BEGIN IMMEDIATE")
    try:
        database.execute(query(sql_name), tuple(bindings))
    except Exception:
        database.rollback()
        raise
    database.rollback()


def scenario_contributor_ownership(database: sqlite3.Connection) -> dict[str, Any]:
    assertion_succeeds(
        database,
        "folder_manage_assert.sql",
        (f"{OP_FOLDER}:owned", FOLDER_OWNED, ORG_A, SPACE_A, 0, 0, CONTRIBUTOR),
    )

    def other_manage() -> None:
        database.execute(
            query("folder_manage_assert.sql"),
            (f"{OP_FOLDER}:other", FOLDER_OTHER, ORG_A, SPACE_A, 0, 0, CONTRIBUTOR),
        )

    expect_trigger(database, CAS_SENTINEL, other_manage)
    assertion_succeeds(
        database,
        "folder_move_assert.sql",
        (f"{OP_FOLDER}:move_owned", FOLDER_OWNED, ORG_A, SPACE_A, 0, 0, None, None, CONTRIBUTOR),
    )

    def other_move() -> None:
        database.execute(
            query("folder_move_assert.sql"),
            (f"{OP_FOLDER}:move_other", FOLDER_OTHER, ORG_A, SPACE_A, 0, 0, None, None, CONTRIBUTOR),
        )

    expect_trigger(database, CAS_SENTINEL, other_move)

    def update_owned() -> None:
        database.execute(
            query("authority_assert.sql"),
            (f"{OP_FOLDER}:authority", ORG_A, CONTRIBUTOR, 1, 0, "active", 0, 0, 0, 0, "write"),
        )
        database.execute(
            query("space_authority_assert.sql"),
            (f"{OP_FOLDER}:space", SPACE_A, CONTRIBUTOR, ORG_A, 0, 0, "write"),
        )
        database.execute(
            query("folder_manage_assert.sql"),
            (f"{OP_FOLDER}:manage", FOLDER_OWNED, ORG_A, SPACE_A, 0, 0, CONTRIBUTOR),
        )
        database.execute(
            query("folder_update.sql"),
            (FOLDER_OWNED, ORG_A, SPACE_A, 0, "Contributor renamed", 1, '{"share":true}', NOW_MS, OP_FOLDER),
        )
        database.execute(
            query("folder_update_postcondition.sql"),
            (
                f"{OP_FOLDER}:post",
                FOLDER_OWNED,
                ORG_A,
                SPACE_A,
                "Contributor renamed",
                1,
                '{"share":true}',
                1,
                0,
                OP_FOLDER,
            ),
        )
        database.execute(query("assertion_cleanup.sql"), (OP_FOLDER,))

    successful_batch(database, update_owned)
    require(
        database.execute(
            "SELECT created_by_user_id,name,revision FROM folders WHERE id=?", (FOLDER_OWNED,)
        ).fetchone()
        == (CONTRIBUTOR, "Contributor renamed", 1),
        "contributor folder update changed ownership or missed its revision fence",
    )
    return {
        "owned_folder_manage_and_move": "allowed",
        "other_owned_folder_manage_and_move": "denied",
        "ownership_after_update": "stable",
    }


def scenario_retention(database: sqlite3.Connection) -> dict[str, Any]:
    retention_ms = 5_000

    def tombstone() -> None:
        database.execute(
            query("authority_assert.sql"),
            (f"{OP_TOMBSTONE}:authority", ORG_RETENTION, OWNER, 1, 0, "active", 0, 0, 0, 0, "owner"),
        )
        database.execute(query("tombstone.sql"), (ORG_RETENTION, retention_ms, OP_TOMBSTONE))
        database.execute(
            query("tombstone_event_insert.sql"),
            (OP_TOMBSTONE, ORG_RETENTION, OWNER, "tombstoned"),
        )
        database.execute(query("assertion_cleanup.sql"), (OP_TOMBSTONE,))

    successful_batch(database, tombstone)
    tombstoned_at, retention_until = database.execute(
        "SELECT tombstoned_at_ms,retention_until_ms FROM organizations WHERE id=?",
        (ORG_RETENTION,),
    ).fetchone()
    require(
        retention_until - tombstoned_at == retention_ms,
        "database clock did not derive the configured recovery window",
    )
    require(
        database.execute(
            "SELECT occurred_at_ms,retention_until_ms FROM organization_tombstone_events_v1 "
            "WHERE operation_id=?",
            (OP_TOMBSTONE,),
        ).fetchone()
        == (tombstoned_at, retention_until),
        "tombstone event did not copy database-derived organization timestamps",
    )
    require(abs(tombstoned_at - int(time.time()) * 1_000) <= 2_000, "tombstone used a caller clock")

    def recover() -> None:
        database.execute(
            query("authority_assert.sql"),
            (f"{OP_RECOVER}:authority", ORG_RETENTION, OWNER, 1, 0, "tombstoned", 1, 1, 0, 0, "owner"),
        )
        database.execute(
            query("recovery_retention_assert.sql"),
            (f"{OP_RECOVER}:retention", ORG_RETENTION, tombstoned_at),
        )
        database.execute(query("recover.sql"), (ORG_RETENTION, tombstoned_at, OP_RECOVER))
        database.execute(
            query("tombstone_event_insert.sql"),
            (OP_RECOVER, ORG_RETENTION, OWNER, "recovered"),
        )
        database.execute(query("assertion_cleanup.sql"), (OP_RECOVER,))
        database.execute(query("retention_assertion_cleanup.sql"), (OP_RECOVER,))

    successful_batch(database, recover)
    require(
        database.execute("SELECT status FROM organizations WHERE id=?", (ORG_RETENTION,)).fetchone()
        == ("active",),
        "in-window recovery did not commit",
    )
    recovered_at = database.execute(
        "SELECT recovered_at_ms FROM organizations WHERE id=?", (ORG_RETENTION,)
    ).fetchone()[0]
    require(
        database.execute(
            "SELECT occurred_at_ms,retention_until_ms FROM organization_tombstone_events_v1 "
            "WHERE operation_id=?",
            (OP_RECOVER,),
        ).fetchone()
        == (recovered_at, None),
        "recovery event did not copy the database-derived recovery timestamp",
    )
    expired_tombstone = database.execute(
        "SELECT tombstoned_at_ms FROM organizations WHERE id=?", (ORG_EXPIRED,)
    ).fetchone()[0]
    before_events = database.execute(
        "SELECT COUNT(*) FROM organization_tombstone_events_v1 WHERE organization_id=?",
        (ORG_EXPIRED,),
    ).fetchone()[0]

    def expired() -> None:
        database.execute(
            query("recovery_retention_assert.sql"),
            (f"{OP_RECOVER}:expired", ORG_EXPIRED, expired_tombstone),
        )

    expect_trigger(database, RETENTION_SENTINEL, expired)
    require(
        database.execute("SELECT status FROM organizations WHERE id=?", (ORG_EXPIRED,)).fetchone()
        == ("tombstoned",),
        "expired organization recovered",
    )
    require(
        before_events
        == database.execute(
            "SELECT COUNT(*) FROM organization_tombstone_events_v1 WHERE organization_id=?",
            (ORG_EXPIRED,),
        ).fetchone()[0],
        "expired recovery appended history",
    )
    return {
        "retention_deadline_authority": "database_clock_plus_configured_policy",
        "in_window_recovery": "recovered",
        "expired_recovery": "retention_locked_without_mutation",
    }


def scenario_support_snapshot(database: sqlite3.Connection) -> dict[str, Any]:
    findings: list[tuple[Any, ...]] = []

    def audit() -> None:
        database.execute(
            query("support_assert.sql"),
            (f"{OP_SUPPORT}:authority", SUPPORT, ORG_A, SUPPORT_DIGEST, 1, 0),
        )
        for name in ("graph_audit.sql", "graph_audit_selections.sql", "graph_audit_folders.sql"):
            findings.extend(database.execute(query(name), (ORG_A, 100)).fetchall())
        database.execute(query("assertion_cleanup.sql"), (OP_SUPPORT,))
        database.execute(
            query("audit_insert.sql"),
            (
                AUDIT_SUPPORT,
                OP_SUPPORT,
                ORG_A,
                SUPPORT,
                "audit_graph",
                "repair_plan",
                hashlib.sha256(ORG_A.encode()).hexdigest(),
                "allow",
                None,
                NOW_MS,
            ),
        )

    successful_batch(database, audit)
    require(
        ("active_selection_without_membership", OUTSIDER, 1) in findings,
        "support-gated graph snapshot did not return its seeded finding",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM organization_repository_assertions_v1 WHERE id LIKE ?",
            (f"{OP_SUPPORT}:%",),
        ).fetchone()
        == (0,),
        "support assertion capability was not consumed",
    )
    support_fingerprint = hashlib.sha256(SUPPORT_DIGEST.encode()).hexdigest()

    def insert_plan() -> None:
        database.execute(
            query("support_assert.sql"),
            (f"{OP_PLAN}:plan_insert", SUPPORT, ORG_A, SUPPORT_DIGEST, 1, 0),
        )
        database.execute(
            query("repair_plan_insert.sql"),
            (PLAN_ID, ORG_A, SUPPORT, support_fingerprint, "[]", "[]", NOW_MS),
        )
        database.execute(query("assertion_cleanup.sql"), (OP_PLAN,))
        database.execute(
            query("audit_insert.sql"),
            (
                AUDIT_PLAN,
                OP_PLAN,
                ORG_A,
                SUPPORT,
                "repair_plan",
                "repair_plan",
                hashlib.sha256(ORG_A.encode()).hexdigest(),
                "allow",
                None,
                NOW_MS,
            ),
        )

    successful_batch(database, insert_plan)
    stored_fingerprint = database.execute(
        "SELECT support_authority_fingerprint FROM organization_repair_plans_v1 WHERE id=?",
        (PLAN_ID,),
    ).fetchone()[0]
    require(
        stored_fingerprint == support_fingerprint and stored_fingerprint != SUPPORT_DIGEST,
        "repair plan retained bearer-equivalent support ticket material",
    )
    database.execute(
        "UPDATE organization_support_authorities_v1 SET revoked_at_ms=? "
        "WHERE support_actor_id=? AND organization_id=? AND ticket_digest=?",
        (NOW_MS, SUPPORT, ORG_A, SUPPORT_DIGEST),
    )
    database.commit()
    before_audits = database.execute(
        "SELECT COUNT(*) FROM organization_audit_events_v1"
    ).fetchone()[0]

    def revoked() -> None:
        database.execute(
            query("support_assert.sql"),
            (f"{OP_SUPPORT}:revoked", SUPPORT, ORG_A, SUPPORT_DIGEST, 1, 0),
        )

    expect_trigger(database, CAS_SENTINEL, revoked)
    require(
        before_audits
        == database.execute("SELECT COUNT(*) FROM organization_audit_events_v1").fetchone()[0],
        "revoked support authority appended a tenant audit row",
    )

    def deleted_organization() -> None:
        database.execute(
            query("support_assert.sql"),
            (f"{OP_SUPPORT}:deleted", SUPPORT, ORG_DELETED, SUPPORT_DIGEST, 1, 0),
        )

    expect_trigger(database, CAS_SENTINEL, deleted_organization)

    source = REPOSITORY_SOURCE.read_text(encoding="utf-8")
    start = source.index("async fn authorized_graph_findings")
    body = source[start : source.index("async fn audit_graph_inner", start)]
    positions = [
        body.index("self.support_statement"),
        body.index("GRAPH_AUDIT_SQL"),
        body.index("GRAPH_AUDIT_SELECTIONS_SQL"),
        body.index("GRAPH_AUDIT_FOLDERS_SQL"),
        body.index("ASSERTION_CLEANUP_SQL"),
        body.index("self.batch_results(statements)"),
    ]
    require(positions == sorted(positions), "support snapshot batch ordering drifted")
    plan_start = source.index("async fn plan_repair_inner")
    plan_body = source[plan_start : source.index("fn support_statement", plan_start)]
    require(
        plan_body.index("self.support_statement") < plan_body.index("REPAIR_PLAN_INSERT_SQL")
        < plan_body.index("self.batch(statements)"),
        "repair plan does not reassert support in its insert batch",
    )
    return {
        "authorized_graph_snapshot": "single_batch",
        "assertion_rows_after_snapshot": 0,
        "revoked_support": "denied_without_tenant_audit_mutation",
        "deleted_organization_support": "denied_before_graph_read",
        "repair_insert": "support_reasserted",
        "repair_plan_support_material": "one_way_fingerprint_only",
    }


def read_authorized(
    database: sqlite3.Connection,
    organization_id: str,
    actor_id: str,
    role_class: str,
    sql_name: str,
    bindings: Sequence[object],
    *,
    session_version: int = 0,
) -> list[tuple[Any, ...]] | None:
    assertion_scope = "read-" + hashlib.sha256(
        repr((organization_id, actor_id, role_class, sql_name, bindings)).encode()
    ).hexdigest()[:24]
    database.execute("BEGIN IMMEDIATE")
    try:
        database.execute(
            query("read_authority_assert.sql"),
            (
                f"{assertion_scope}:read",
                organization_id,
                actor_id,
                1,
                session_version,
                role_class,
            ),
        )
    except sqlite3.IntegrityError as error:
        database.rollback()
        require(CAS_SENTINEL in str(error), "collection authority trigger drifted")
        return None
    try:
        rows = database.execute(query(sql_name), tuple(bindings)).fetchall()
        database.execute(query("assertion_cleanup.sql"), (assertion_scope,))
    except Exception:
        database.rollback()
        raise
    database.commit()
    return rows


def scenario_list_manage_share(database: sqlite3.Connection) -> dict[str, Any]:
    source = REPOSITORY_SOURCE.read_text(encoding="utf-8")
    require(
        "batch_results(vec![authority_statement, data, cleanup])" in source,
        "collection data is no longer sequenced after its authority assertion",
    )
    first = read_authorized(database, ORG_A, CONTRIBUTOR, "any", "members_list.sql", (ORG_A, None, 3))
    require(first is not None and len(first) == 3, "member list first page drifted")
    cursor = first[1][1]
    second = read_authorized(database, ORG_A, CONTRIBUTOR, "any", "members_list.sql", (ORG_A, cursor, 3))
    require(
        second is not None and all(row[1] > cursor for row in second),
        "member list cursor was not exclusive",
    )
    require(
        read_authorized(database, ORG_A, OUTSIDER, "any", "members_list.sql", (ORG_A, None, 3))
        is None,
        "cross-tenant actor received a collection",
    )
    invites = read_authorized(database, ORG_A, ADMIN, "admin", "invites_list.sql", (ORG_A, None, 10))
    require(invites is not None and len(invites) == 2, "admin invite listing drifted")
    require(
        read_authorized(database, ORG_A, CONTRIBUTOR, "admin", "invites_list.sql", (ORG_A, None, 10))
        is None,
        "member listed privileged invite metadata",
    )
    require(
        "digest" not in query("invites_list.sql").lower(),
        "invite list exposes an email or token digest",
    )
    for sql_name, bindings, minimum in (
        ("domains_list.sql", (ORG_A, None, 10), 2),
        ("spaces_list.sql", (ORG_A, None, 10), 1),
        ("space_members_list.sql", (ORG_A, SPACE_A, None, 10), 2),
        ("folders_list.sql", (ORG_A, SPACE_A, None, 10), 2),
    ):
        rows = read_authorized(database, ORG_A, OWNER, "any", sql_name, bindings)
        require(rows is not None and len(rows) >= minimum, f"authorized {sql_name} listing drifted")

    def create_public_space() -> None:
        database.execute(
            query("authority_assert.sql"),
            (
                f"{OP_CREATE_SPACE}:authority",
                ORG_A,
                CONTRIBUTOR,
                1,
                0,
                "active",
                0,
                0,
                0,
                0,
                "write",
            ),
        )
        database.execute(
            query("space_insert.sql"),
            (SPACE_CREATED, ORG_A, CONTRIBUTOR, "Created public", 0, 1, "{}", NOW_MS, OP_CREATE_SPACE),
        )
        database.execute(
            query("space_role_upsert.sql"),
            (SPACE_CREATED, CONTRIBUTOR, "manager", "active", NOW_MS, OP_CREATE_SPACE, ORG_A, -1),
        )
        database.execute(
            query("space_role_postcondition.sql"),
            (
                f"{OP_CREATE_SPACE}:creator_post",
                SPACE_CREATED,
                CONTRIBUTOR,
                ORG_A,
                "manager",
                "active",
                0,
                OP_CREATE_SPACE,
            ),
        )
        database.execute(
            query("space_postcondition.sql"),
            (f"{OP_CREATE_SPACE}:post", SPACE_CREATED, ORG_A, 0, OP_CREATE_SPACE),
        )
        database.execute(query("assertion_cleanup.sql"), (OP_CREATE_SPACE,))

    successful_batch(database, create_public_space)
    require(
        database.execute(
            "SELECT s.is_public,sm.role,sm.state FROM spaces s "
            "JOIN space_members sm ON sm.space_id=s.id AND sm.user_id=s.created_by_user_id "
            "WHERE s.id=? AND s.organization_id=?",
            (SPACE_CREATED, ORG_A),
        ).fetchone()
        == (1, "manager", "active"),
        "space creator did not receive atomic manager authority",
    )

    def update_space() -> None:
        database.execute(
            query("authority_assert.sql"),
            (f"{OP_SPACE}:authority", ORG_A, ADMIN, 1, 0, "active", 0, 0, 0, 0, "write"),
        )
        database.execute(
            query("space_authority_assert.sql"),
            (f"{OP_SPACE}:space", SPACE_A, ADMIN, ORG_A, 0, None, "manager"),
        )
        database.execute(
            query("space_update.sql"),
            (SPACE_A, ORG_A, 0, "Managed space", 1, '{"managed":true}', NOW_MS, OP_SPACE),
        )
        database.execute(
            query("space_update_postcondition.sql"),
            (f"{OP_SPACE}:post", SPACE_A, ORG_A, "Managed space", 1, '{"managed":true}', 1, OP_SPACE),
        )
        database.execute(query("assertion_cleanup.sql"), (OP_SPACE,))

    successful_batch(database, update_space)

    def contributor_manage_space() -> None:
        database.execute(
            query("space_authority_assert.sql"),
            (f"{OP_SPACE}:contributor", SPACE_A, CONTRIBUTOR, ORG_A, 1, 0, "manager"),
        )

    expect_trigger(database, CAS_SENTINEL, contributor_manage_space)

    def share_space() -> None:
        database.execute(
            query("authority_assert.sql"),
            (f"{OP_SHARE}:authority", ORG_A, ADMIN, 1, 0, "active", 0, 0, 0, 0, "write"),
        )
        database.execute(
            query("space_authority_assert.sql"),
            (f"{OP_SHARE}:space", SPACE_A, ADMIN, ORG_A, 1, None, "manager"),
        )
        database.execute(
            query("space_role_upsert.sql"),
            (SPACE_A, VIEWER, "contributor", "active", NOW_MS, OP_SHARE, ORG_A, 0),
        )
        database.execute(query("principal_authority_bump.sql"), (VIEWER, NOW_MS, OP_SHARE))
        database.execute(query("principal_grants_revoke.sql"), (VIEWER,))
        database.execute(
            query("principal_bump_postcondition.sql"),
            (f"{OP_SHARE}:principal_post", VIEWER, OP_SHARE),
        )
        database.execute(
            query("space_role_postcondition.sql"),
            (f"{OP_SHARE}:post", SPACE_A, VIEWER, ORG_A, "contributor", "active", 1, OP_SHARE),
        )
        database.execute(query("assertion_cleanup.sql"), (OP_SHARE,))

    successful_batch(database, share_space)
    require(
        database.execute(
            "SELECT sm.role,sm.revision,i.session_version FROM space_members sm "
            "JOIN auth_identities_v2 i ON i.user_id=sm.user_id "
            "WHERE sm.space_id=? AND sm.user_id=?",
            (SPACE_A, VIEWER),
        ).fetchone()
        == ("contributor", 1, 1),
        "space share did not update role and invalidate prior authority",
    )

    def share_domain() -> None:
        database.execute(
            query("authority_assert.sql"),
            (f"{OP_DOMAIN}:authority", ORG_A, ADMIN, 1, 0, "active", 0, 0, 0, 0, "admin"),
        )
        database.execute(
            query("domain_capacity_assert.sql"),
            (f"{OP_DOMAIN}:capacity", ORG_A, "gamma.example"),
        )
        database.execute(
            query("domain_upsert.sql"),
            (ORG_A, "gamma.example", NOW_MS, NOW_MS, OP_DOMAIN, -1),
        )
        database.execute(
            query("domain_postcondition.sql"),
            (f"{OP_DOMAIN}:post", ORG_A, "gamma.example", 0, OP_DOMAIN),
        )
        database.execute(query("assertion_cleanup.sql"), (OP_DOMAIN,))

    successful_batch(database, share_domain)
    require(
        database.execute(
            "SELECT COUNT(*) FROM organization_allowed_domains WHERE organization_id=?",
            (ORG_A,),
        ).fetchone()
        == (3,),
        "allowed-domain share workflow did not commit",
    )
    return {
        "paginated_collections": "authority_asserted_before_tenant_select_and_cursor_exclusive",
        "privileged_invite_list": "redacted_and_admin_only",
        "space_manage": "admin_allowed_contributor_denied",
        "public_space_creation": "member_creator_is_atomic_manager_by_policy",
        "space_share": "role_updated_and_session_invalidated",
        "domain_share": "capacity_fenced",
    }


def artifact_digest(paths: Sequence[pathlib.Path]) -> str:
    digest = hashlib.sha256()
    for path in paths:
        digest.update(path.name.encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def run() -> dict[str, Any]:
    dirty = scenario_dirty_upgrade()
    database = sqlite3.connect(":memory:")
    try:
        migrate(database)
        compile_all_queries(database)
        seed(database)
        scenarios = {
            "dirty_upgrade": dirty,
            "invite_identity": scenario_invite_identity(database),
            "semantic_replay_mismatch": scenario_semantic_replay(database),
            "denial_nonmutation": scenario_denial_nonmutation(database),
            "contributor_ownership": scenario_contributor_ownership(database),
            "retention": scenario_retention(database),
            "support_snapshot_cleanup": scenario_support_snapshot(database),
            "list_manage_share": scenario_list_manage_share(database),
        }
        require(database.execute("PRAGMA foreign_key_check").fetchall() == [], "foreign key drift")
        require(
            database.execute("SELECT COUNT(*) FROM organization_repository_assertions_v1").fetchone()
            == (0,),
            "repository assertion capability leaked after the suite",
        )
        require(
            database.execute("SELECT COUNT(*) FROM organization_retention_assertions_v1").fetchone()
            == (0,),
            "retention assertion capability leaked after the suite",
        )
    finally:
        database.close()
    migrations = migration_files()
    queries = sorted(QUERIES.glob("*.sql"))
    return {
        "schema_version": 1,
        "suite": "frame-organization-sqlite-semantic-conformance",
        "runtime_boundary": "python_sqlite3_checked_in_migrations_and_queries",
        "network_used": False,
        "migration_count": len(migrations),
        "migration_digest_sha256": artifact_digest(migrations),
        "query_count": len(queries),
        "query_digest_sha256": artifact_digest(queries),
        "scenarios": scenarios,
        "wrangler_artifact_status": {
            "path": "target/evidence/organization-d1-conformance.json",
            "status": "stale_pre_repair",
            "validates_current_repairs": False,
        },
        "not_claimed": [
            "compiled_rust_wasm_worker_execution",
            "wrangler_or_d1_provider_parity",
            "remote_d1_contention_or_replication",
            "production_rollout_or_security_signoff",
        ],
        "result": "pass",
    }


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--evidence",
        type=pathlib.Path,
        default=ROOT / "target" / "evidence" / "organization-sqlite-semantic-conformance.json",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        report = run()
    except (AssertionError, ConformanceFailure, OSError, sqlite3.Error) as error:
        print(f"organization SQLite semantic conformance failed: {error}", file=sys.stderr)
        return 1
    arguments.evidence.parent.mkdir(parents=True, exist_ok=True)
    arguments.evidence.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print("organization SQLite semantic conformance passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
