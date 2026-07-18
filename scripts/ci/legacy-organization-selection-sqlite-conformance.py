#!/usr/bin/env python3
"""SQLite proof for the active-only legacy organization-selection D1 batch."""

from __future__ import annotations

import hashlib
import sqlite3
import uuid
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
QUERY_ROOT = ROOT / "apps/control-plane/queries/organization"
AUTH_QUERY_ROOT = ROOT / "apps/control-plane/queries/auth"


def query(name: str) -> str:
    return (QUERY_ROOT / name).read_text(encoding="utf-8").strip()


AUTHORITY_ASSERT = query("legacy_selection_authority_assert.sql")
ACTIVE_UPDATE = query("legacy_selection_active_update.sql")
POSTCONDITION = query("legacy_selection_postcondition.sql")
OPERATION_INSERT = query("legacy_selection_operation_insert.sql")
RECEIPT = query("legacy_selection_receipt.sql")
CLEANUP = query("assertion_cleanup.sql")
AUDIT_INSERT = query("audit_insert.sql")
GRANT_ASSERT = (AUTH_QUERY_ROOT / "browser_mutation_grant_assert.sql").read_text(
    encoding="utf-8"
).strip()
GRANT_DELETE = (
    AUTH_QUERY_ROOT / "browser_mutation_grant_delete_by_proof.sql"
).read_text(encoding="utf-8").strip()
GRANT_CHANGE_ASSERT = (
    AUTH_QUERY_ROOT / "browser_mutation_change_assert.sql"
).read_text(encoding="utf-8").strip()


@dataclass(frozen=True)
class Selection:
    actor_id: str
    organization_id: str
    authorization: str
    lifecycle: str
    occurred_at_ms: int


@dataclass(frozen=True)
class BrowserProof:
    grant_id: str
    session_id: str
    user_id: str


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:")
    connection.row_factory = sqlite3.Row
    connection.executescript(
        """
        PRAGMA foreign_keys = ON;
        CREATE TABLE users (
          id TEXT PRIMARY KEY,
          status TEXT NOT NULL,
          default_organization_id TEXT,
          active_organization_id TEXT,
          organization_preference_revision INTEGER NOT NULL DEFAULT 0,
          organization_last_operation_id TEXT,
          updated_at_ms INTEGER NOT NULL,
          deleted_at_ms INTEGER
        );
        CREATE TABLE auth_identities_v2 (
          user_id TEXT PRIMARY KEY,
          session_version INTEGER NOT NULL
        );
        CREATE TABLE auth_sessions_v2 (
          id TEXT PRIMARY KEY,
          user_id TEXT NOT NULL,
          state TEXT NOT NULL,
          generation INTEGER NOT NULL,
          token_key_version INTEGER NOT NULL,
          token_digest TEXT NOT NULL,
          session_version INTEGER NOT NULL,
          idle_expires_at_ms INTEGER NOT NULL,
          absolute_expires_at_ms INTEGER NOT NULL
        );
        CREATE TABLE auth_session_mutation_grants_v2 (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          user_id TEXT NOT NULL,
          generation INTEGER NOT NULL,
          token_key_version INTEGER NOT NULL,
          token_digest TEXT NOT NULL
        );
        CREATE TABLE authenticated_web_action_assertions_v1 (
          operation_id TEXT NOT NULL,
          assertion_kind TEXT NOT NULL,
          expected_count INTEGER NOT NULL,
          actual_count INTEGER NOT NULL,
          PRIMARY KEY (operation_id, assertion_kind),
          CHECK (expected_count = actual_count)
        );
        CREATE TABLE organizations (
          id TEXT PRIMARY KEY,
          owner_id TEXT NOT NULL,
          status TEXT NOT NULL,
          authority_version INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE organization_members (
          organization_id TEXT NOT NULL,
          user_id TEXT NOT NULL,
          state TEXT NOT NULL,
          PRIMARY KEY (organization_id, user_id)
        );
        CREATE TABLE organization_repository_assertions_v1 (
          id TEXT PRIMARY KEY,
          satisfied INTEGER NOT NULL CHECK (satisfied = 1)
        );
        CREATE TRIGGER organization_repository_assertions_v1_conflict
        BEFORE INSERT ON organization_repository_assertions_v1
        WHEN NEW.satisfied <> 1
        BEGIN
          SELECT RAISE(ABORT, 'frame_organization_cas_conflict_v1');
        END;
        CREATE TABLE organization_repository_operations_v1 (
          operation_id TEXT PRIMARY KEY,
          organization_id TEXT NOT NULL,
          idempotency_key TEXT NOT NULL,
          operation_kind TEXT NOT NULL,
          subject_id TEXT NOT NULL,
          request_fingerprint TEXT NOT NULL,
          result_code TEXT NOT NULL,
          resulting_revision INTEGER NOT NULL,
          authority_version INTEGER NOT NULL,
          committed_at_ms INTEGER NOT NULL,
          UNIQUE (organization_id, idempotency_key)
        );
        CREATE TABLE organization_audit_events_v1 (
          id TEXT PRIMARY KEY,
          operation_id TEXT NOT NULL,
          organization_id TEXT NOT NULL,
          actor_id TEXT NOT NULL,
          action TEXT NOT NULL,
          subject_kind TEXT NOT NULL,
          subject_digest TEXT NOT NULL,
          outcome TEXT NOT NULL,
          denial_code TEXT,
          occurred_at_ms INTEGER NOT NULL,
          metadata_json TEXT NOT NULL
        );
        """
    )
    return connection


def seed(
    connection: sqlite3.Connection,
    *,
    actor_id: str = "actor-a",
    organization_id: str = "organization-a",
    owner: bool = False,
    membership: bool = False,
    organization_status: str = "active",
) -> None:
    connection.execute(
        "INSERT INTO users VALUES (?, 'active', 'default-organization', NULL, 7, NULL, 1, NULL)",
        (actor_id,),
    )
    connection.execute(
        "INSERT INTO organizations VALUES (?, ?, ?, 13)",
        (
            organization_id,
            actor_id if owner else "different-owner",
            organization_status,
        ),
    )
    if membership:
        connection.execute(
            "INSERT INTO organization_members VALUES (?, ?, 'active')",
            (organization_id, actor_id),
        )
    connection.commit()


def seed_browser_proof(connection: sqlite3.Connection) -> BrowserProof:
    proof = BrowserProof(
        grant_id="018f6f65-7d5d-7d46-a3e1-4e7da76f36a9",
        session_id="018f6f65-7d5d-7d46-a3e1-4e7da76f36aa",
        user_id="actor-a",
    )
    token_digest = "a" * 64
    connection.execute(
        "INSERT INTO auth_identities_v2 VALUES (?, 3)",
        (proof.user_id,),
    )
    connection.execute(
        "INSERT INTO auth_sessions_v2 VALUES (?, ?, 'active', 4, 7, ?, 3, ?, ?)",
        (
            proof.session_id,
            proof.user_id,
            token_digest,
            1_700_000_060_000,
            1_700_003_600_000,
        ),
    )
    connection.execute(
        "INSERT INTO auth_session_mutation_grants_v2 VALUES (?, ?, ?, 4, 7, ?)",
        (proof.grant_id, proof.session_id, proof.user_id, token_digest),
    )
    connection.commit()
    return proof


def execute(
    connection: sqlite3.Connection,
    selection: Selection,
    proof: BrowserProof | None = None,
) -> sqlite3.Row | None:
    operation_id = str(uuid.uuid4())
    idempotency_key = f"legacy-auto-{operation_id}"
    fingerprint = hashlib.sha256(
        (
            "active_organization_set\0"
            f"{selection.actor_id}\0{selection.organization_id}\0"
            f"{selection.authorization}\0{selection.lifecycle}"
        ).encode()
    ).hexdigest()
    subject_digest = hashlib.sha256(selection.actor_id.encode()).hexdigest()
    try:
        with connection:
            if proof is not None:
                connection.execute(
                    GRANT_ASSERT,
                    (
                        operation_id,
                        proof.grant_id,
                        proof.session_id,
                        proof.user_id,
                        selection.occurred_at_ms,
                    ),
                )
            connection.execute(
                AUTHORITY_ASSERT,
                (
                    f"{operation_id}:legacy_authority",
                    selection.actor_id,
                    selection.organization_id,
                    selection.lifecycle,
                    selection.authorization,
                ),
            )
            connection.execute(
                ACTIVE_UPDATE,
                (
                    selection.actor_id,
                    selection.organization_id,
                    operation_id,
                    selection.occurred_at_ms,
                ),
            )
            connection.execute(
                POSTCONDITION,
                (
                    f"{operation_id}:legacy_post",
                    selection.actor_id,
                    selection.organization_id,
                    operation_id,
                ),
            )
            connection.execute(
                OPERATION_INSERT,
                (
                    operation_id,
                    selection.organization_id,
                    idempotency_key,
                    selection.actor_id,
                    fingerprint,
                    selection.occurred_at_ms,
                ),
            )
            if proof is not None:
                connection.execute(
                    GRANT_DELETE,
                    (proof.grant_id, proof.session_id, proof.user_id),
                )
                connection.execute(
                    GRANT_CHANGE_ASSERT,
                    (operation_id, "grant_consumed"),
                )
                connection.execute(
                    "DELETE FROM authenticated_web_action_assertions_v1 "
                    "WHERE operation_id = ?",
                    (operation_id,),
                )
            connection.execute(CLEANUP, (operation_id,))
            connection.execute(
                AUDIT_INSERT,
                (
                    str(uuid.uuid4()),
                    operation_id,
                    selection.organization_id,
                    selection.actor_id,
                    "active_organization_set",
                    "organization",
                    subject_digest,
                    "allow",
                    None,
                    selection.occurred_at_ms,
                ),
            )
    except sqlite3.IntegrityError as error:
        assert (
            "frame_organization_cas_conflict_v1" in str(error)
            or "CHECK constraint failed" in str(error)
        ), error
        return None
    return connection.execute(RECEIPT, (operation_id,)).fetchone()


def selection(*, authorization: str, lifecycle: str) -> Selection:
    return Selection(
        actor_id="actor-a",
        organization_id="organization-a",
        authorization=authorization,
        lifecycle=lifecycle,
        occurred_at_ms=1_700_000_000_000,
    )


def consume_browser_proof(
    connection: sqlite3.Connection,
    proof: BrowserProof,
    now_ms: int,
) -> bool:
    operation_id = str(uuid.uuid4())
    try:
        with connection:
            connection.execute(
                GRANT_ASSERT,
                (
                    operation_id,
                    proof.grant_id,
                    proof.session_id,
                    proof.user_id,
                    now_ms,
                ),
            )
            connection.execute(
                GRANT_DELETE,
                (proof.grant_id, proof.session_id, proof.user_id),
            )
            connection.execute(
                GRANT_CHANGE_ASSERT,
                (operation_id, "grant_consumed"),
            )
            connection.execute(
                "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id = ?",
                (operation_id,),
            )
    except sqlite3.IntegrityError:
        return False
    return True


def test_web_membership_updates_only_active_and_derives_revision() -> None:
    connection = database()
    seed(connection, membership=True, organization_status="tombstoned")
    receipt = execute(
        connection,
        selection(authorization="active_membership", lifecycle="any"),
    )
    assert receipt is not None
    row = connection.execute(
        "SELECT default_organization_id, active_organization_id, organization_preference_revision "
        "FROM users WHERE id = 'actor-a'"
    ).fetchone()
    assert tuple(row) == ("default-organization", "organization-a", 8)
    assert receipt["resulting_revision"] == 8
    assert receipt["authority_version"] == 13
    assert receipt["operation_kind"] == "active_organization_set"
    assert connection.execute(
        "SELECT COUNT(*) FROM organization_audit_events_v1 WHERE outcome = 'allow'"
    ).fetchone()[0] == 1
    assert connection.execute(
        "SELECT action FROM organization_audit_events_v1"
    ).fetchone()[0] == "active_organization_set"


def test_web_owner_without_membership_is_denied_without_partial_write() -> None:
    connection = database()
    seed(connection, owner=True, membership=False)
    assert execute(
        connection,
        selection(authorization="active_membership", lifecycle="any"),
    ) is None
    row = connection.execute(
        "SELECT default_organization_id, active_organization_id, organization_preference_revision "
        "FROM users WHERE id = 'actor-a'"
    ).fetchone()
    assert tuple(row) == ("default-organization", None, 7)
    assert connection.execute(
        "SELECT COUNT(*) FROM organization_repository_operations_v1"
    ).fetchone()[0] == 0


def test_mobile_owner_fallback_is_allowed_only_for_active_organization() -> None:
    connection = database()
    seed(connection, owner=True, membership=False)
    assert execute(
        connection,
        selection(
            authorization="owner_or_active_membership", lifecycle="active_only"
        ),
    ) is not None

    tombstoned = database()
    seed(tombstoned, owner=True, membership=False, organization_status="tombstoned")
    assert execute(
        tombstoned,
        selection(
            authorization="owner_or_active_membership", lifecycle="active_only"
        ),
    ) is None


def test_same_target_retry_executes_again_and_preserves_default() -> None:
    connection = database()
    seed(connection, membership=True)
    command = selection(authorization="active_membership", lifecycle="any")
    first = execute(connection, command)
    second = execute(connection, command)
    assert first is not None and second is not None
    assert first["operation_id"] != second["operation_id"]
    assert (first["resulting_revision"], second["resulting_revision"]) == (8, 9)
    row = connection.execute(
        "SELECT default_organization_id, active_organization_id, organization_preference_revision "
        "FROM users WHERE id = 'actor-a'"
    ).fetchone()
    assert tuple(row) == ("default-organization", "organization-a", 9)


def test_query_shape_cannot_overwrite_default_or_require_client_revision() -> None:
    normalized = " ".join(ACTIVE_UPDATE.lower().split())
    assert "default_organization_id" not in normalized
    assert "organization_preference_revision = organization_preference_revision + 1" in normalized
    assert "organization_preference_revision = ?" not in normalized
    assert "u.organization_preference_revision" in OPERATION_INSERT


def test_browser_proof_is_consumed_atomically_with_selection_and_journals() -> None:
    connection = database()
    seed(connection, membership=True)
    proof = seed_browser_proof(connection)
    receipt = execute(
        connection,
        selection(authorization="active_membership", lifecycle="any"),
        proof,
    )
    assert receipt is not None
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2"
    ).fetchone()[0] == 0
    assert connection.execute(
        "SELECT active_organization_id FROM users WHERE id='actor-a'"
    ).fetchone()[0] == "organization-a"
    assert connection.execute(
        "SELECT COUNT(*) FROM organization_repository_operations_v1"
    ).fetchone()[0] == 1
    assert connection.execute(
        "SELECT COUNT(*) FROM organization_audit_events_v1"
    ).fetchone()[0] == 1
    assert connection.execute(
        "SELECT COUNT(*) FROM authenticated_web_action_assertions_v1"
    ).fetchone()[0] == 0


def test_stale_or_missing_browser_proof_rolls_back_every_selection_effect() -> None:
    for stale_kind in ("replaced", "missing"):
        connection = database()
        seed(connection, membership=True)
        proof = seed_browser_proof(connection)
        if stale_kind == "replaced":
            connection.execute(
                "UPDATE auth_sessions_v2 SET generation = generation + 1 WHERE id = ?",
                (proof.session_id,),
            )
        else:
            connection.execute(
                "DELETE FROM auth_session_mutation_grants_v2 WHERE id = ?",
                (proof.grant_id,),
            )
        connection.commit()
        assert execute(
            connection,
            selection(authorization="active_membership", lifecycle="any"),
            proof,
        ) is None
        user = connection.execute(
            "SELECT active_organization_id,organization_preference_revision "
            "FROM users WHERE id='actor-a'"
        ).fetchone()
        assert tuple(user) == (None, 7)
        assert connection.execute(
            "SELECT COUNT(*) FROM organization_repository_operations_v1"
        ).fetchone()[0] == 0
        assert connection.execute(
            "SELECT COUNT(*) FROM organization_audit_events_v1"
        ).fetchone()[0] == 0


def test_denied_target_rolls_back_then_proof_is_consumed_without_mutation() -> None:
    connection = database()
    seed(connection, membership=False)
    proof = seed_browser_proof(connection)
    command = selection(authorization="active_membership", lifecycle="any")
    assert execute(connection, command, proof) is None
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2"
    ).fetchone()[0] == 1
    assert consume_browser_proof(connection, proof, command.occurred_at_ms)
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2"
    ).fetchone()[0] == 0
    user = connection.execute(
        "SELECT active_organization_id,organization_preference_revision "
        "FROM users WHERE id='actor-a'"
    ).fetchone()
    assert tuple(user) == (None, 7)
    assert connection.execute(
        "SELECT COUNT(*) FROM organization_repository_operations_v1"
    ).fetchone()[0] == 0
    assert connection.execute(
        "SELECT COUNT(*) FROM organization_audit_events_v1"
    ).fetchone()[0] == 0


def main() -> None:
    tests = [
        test_web_membership_updates_only_active_and_derives_revision,
        test_web_owner_without_membership_is_denied_without_partial_write,
        test_mobile_owner_fallback_is_allowed_only_for_active_organization,
        test_same_target_retry_executes_again_and_preserves_default,
        test_query_shape_cannot_overwrite_default_or_require_client_revision,
        test_browser_proof_is_consumed_atomically_with_selection_and_journals,
        test_stale_or_missing_browser_proof_rolls_back_every_selection_effect,
        test_denied_target_rolls_back_then_proof_is_consumed_without_mutation,
    ]
    for test in tests:
        test()
    print(f"legacy organization selection SQLite conformance: {len(tests)} passed")


if __name__ == "__main__":
    main()
