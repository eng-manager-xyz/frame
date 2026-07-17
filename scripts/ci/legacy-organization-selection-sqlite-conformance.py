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


def query(name: str) -> str:
    return (QUERY_ROOT / name).read_text(encoding="utf-8").strip()


AUTHORITY_ASSERT = query("legacy_selection_authority_assert.sql")
ACTIVE_UPDATE = query("legacy_selection_active_update.sql")
POSTCONDITION = query("legacy_selection_postcondition.sql")
OPERATION_INSERT = query("legacy_selection_operation_insert.sql")
RECEIPT = query("legacy_selection_receipt.sql")
CLEANUP = query("assertion_cleanup.sql")
AUDIT_INSERT = query("audit_insert.sql")


@dataclass(frozen=True)
class Selection:
    actor_id: str
    organization_id: str
    authorization: str
    lifecycle: str
    occurred_at_ms: int


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
          updated_at_ms INTEGER NOT NULL
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
        "INSERT INTO users VALUES (?, 'active', 'default-organization', NULL, 7, NULL, 1)",
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


def execute(connection: sqlite3.Connection, selection: Selection) -> sqlite3.Row | None:
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
            connection.execute(CLEANUP, (operation_id,))
            connection.execute(
                AUDIT_INSERT,
                (
                    str(uuid.uuid4()),
                    operation_id,
                    selection.organization_id,
                    selection.actor_id,
                    "organization_read",
                    "organization",
                    subject_digest,
                    "allow",
                    None,
                    selection.occurred_at_ms,
                ),
            )
    except sqlite3.IntegrityError as error:
        assert "frame_organization_cas_conflict_v1" in str(error), error
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
    ).fetchone()[0] == "organization_read"


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


def main() -> None:
    tests = [
        test_web_membership_updates_only_active_and_derives_revision,
        test_web_owner_without_membership_is_denied_without_partial_write,
        test_mobile_owner_fallback_is_allowed_only_for_active_organization,
        test_same_target_retry_executes_again_and_preserves_default,
        test_query_shape_cannot_overwrite_default_or_require_client_revision,
    ]
    for test in tests:
        test()
    print(f"legacy organization selection SQLite conformance: {len(tests)} passed")


if __name__ == "__main__":
    main()
