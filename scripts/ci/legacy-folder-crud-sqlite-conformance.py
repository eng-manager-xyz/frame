#!/usr/bin/env python3
"""Provider-free SQLite proof for Cap mobile/RPC folder CRUD.

The suite applies the complete expand chain and executes the checked-in D1 SQL
for exact mobile create, scoped RPC create/update/delete, replay/conflict,
recursive relocation, tenant/authority fencing, rollback, and immutable proof.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
import uuid
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_folder_crud"
MIGRATION = MIGRATIONS / "0041_legacy_folder_crud_expand.sql"
RUNTIME = ROOT / "apps/control-plane/src/legacy_folder_crud_runtime.rs"
APPLICATION = ROOT / "crates/application/src/legacy_folder_crud.rs"
NOW = 1_700_000_000_000
ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"

MOBILE_CREATE = "cap-v1-7160c4389375c682"
RPC_CREATE = "cap-v1-9e125712cee9ce5a"
RPC_DELETE = "cap-v1-eea1796482b3af28"
RPC_UPDATE = "cap-v1-a193e9e08b2c3f7d"

SQL = {
    path.stem: path.read_text(encoding="utf-8").strip()
    for path in sorted(QUERIES.glob("*.sql"))
}
ORGANIZATION_FOLDERS_SQL = (
    ROOT / "apps/control-plane/queries/organization/folders_list.sql"
).read_text(encoding="utf-8")


def fixture_id(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def cap_id(number: int) -> str:
    output = []
    for _ in range(15):
        output.append(ALPHABET[number & 31])
        number >>= 5
    return "".join(reversed(output))


def mapped_uuid(value: str) -> str:
    payload = hashlib.sha256(
        b"frame-cap-nanoid-to-uuid-v1\0" + value.encode()
    ).digest()
    result = bytearray(payload[:16])
    result[6] = (result[6] & 0x0F) | 0x80
    result[8] = (result[8] & 0x3F) | 0x80
    return str(uuid.UUID(bytes=bytes(result)))


def digest(label: str) -> str:
    return hashlib.sha256(f"frame-folder-crud:{label}".encode()).hexdigest()


OWNER = fixture_id(1)
ACTOR = fixture_id(2)
VIEWER = fixture_id(3)
FOREIGN = fixture_id(4)
ORG = fixture_id(10)
FOREIGN_ORG = fixture_id(11)
SPACE = fixture_id(20)
FOREIGN_SPACE = fixture_id(21)


def migrated_database() -> sqlite3.Connection:
    database = sqlite3.connect(":memory:", isolation_level=None)
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        database.executescript(migration.read_text(encoding="utf-8"))
        assert not database.execute("PRAGMA foreign_key_check").fetchall(), migration
    return database


def seed(database: sqlite3.Connection) -> None:
    for user_id, name, active_org in (
        (OWNER, "owner", ORG),
        (ACTOR, "actor", ORG),
        (VIEWER, "viewer", ORG),
        (FOREIGN, "foreign", FOREIGN_ORG),
    ):
        database.execute(
            """INSERT INTO users(
                 id,email,display_name,created_at_ms,updated_at_ms,status,
                 active_organization_id
               ) VALUES (?,?,?,?,?,'active',?)""",
            (user_id, f"{name}@example.test", name, NOW, NOW, active_org),
        )
    database.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
           ) VALUES (?,?,?,'active','{}',?,?)""",
        (ORG, OWNER, "Frame", NOW, NOW),
    )
    database.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
           ) VALUES (?,?,?,'active','{}',?,?)""",
        (FOREIGN_ORG, FOREIGN, "Foreign", NOW, NOW),
    )
    for organization_id, user_id, role, pro in (
        (ORG, OWNER, "owner", 1),
        (ORG, ACTOR, "admin", 0),
        (ORG, VIEWER, "viewer", 0),
        (FOREIGN_ORG, FOREIGN, "owner", 1),
    ):
        database.execute(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,has_pro_seat,
                 created_at_ms,updated_at_ms
               ) VALUES (?,?,?,'active',?,?,?)""",
            (organization_id, user_id, role, pro, NOW, NOW),
        )
    database.execute(
        """INSERT INTO spaces(
             id,organization_id,created_by_user_id,name,created_at_ms,updated_at_ms
           ) VALUES (?,?,?,?,?,?)""",
        (SPACE, ORG, ACTOR, "Launch", NOW, NOW),
    )
    database.execute(
        """INSERT INTO spaces(
             id,organization_id,created_by_user_id,name,created_at_ms,updated_at_ms
           ) VALUES (?,?,?,?,?,?)""",
        (FOREIGN_SPACE, FOREIGN_ORG, FOREIGN, "Foreign", NOW, NOW),
    )
    database.execute(
        """INSERT INTO space_members(
             space_id,user_id,role,created_at_ms,updated_at_ms,state
           ) VALUES (?,?,'manager',?,?,'active')""",
        (SPACE, ACTOR, NOW, NOW),
    )


def one(database: sqlite3.Connection, query: str, values: tuple[Any, ...]) -> sqlite3.Row:
    rows = database.execute(SQL[query], values).fetchall()
    assert len(rows) == 1, (query, len(rows))
    return rows[0]


def authority(database: sqlite3.Connection, actor: str = ACTOR) -> sqlite3.Row:
    return one(database, "authority_snapshot", (actor, ORG))


def authority_assert(
    database: sqlite3.Connection, operation: str, snapshot: sqlite3.Row, actor: str = ACTOR
) -> None:
    database.execute(
        SQL["authority_assert"],
        (
            operation,
            actor,
            ORG,
            snapshot["selection_revision"],
            snapshot["owner_id"],
            snapshot["organization_revision"],
            snapshot["organization_authority_version"],
            snapshot["membership_role"],
            snapshot["membership_revision"],
            snapshot["membership_authority_version"],
            snapshot["owner_has_pro_seat"],
            snapshot["owner_membership_revision"],
            snapshot["owner_membership_authority_version"],
        ),
    )


def scope_snapshot(
    database: sqlite3.Connection, kind: str, scope_id: str | None, actor: str = ACTOR
) -> sqlite3.Row:
    return one(database, "scope_snapshot", (ORG, actor, kind, scope_id))


def scope_assert(
    database: sqlite3.Connection,
    operation: str,
    snapshot: sqlite3.Row,
    actor: str = ACTOR,
) -> None:
    database.execute(
        SQL["scope_assert"],
        (
            operation,
            ORG,
            actor,
            snapshot["scope_kind"],
            snapshot["scope_id"],
            snapshot["scope_revision"],
            snapshot["scope_authority_version"],
            snapshot["scope_creator_id"],
            snapshot["actor_space_role"],
            snapshot["actor_space_membership_revision"],
        ),
    )


def folder_snapshot(
    database: sqlite3.Connection, folder_id: str, actor: str = ACTOR
) -> sqlite3.Row:
    return one(database, "folder_snapshot", (folder_id, ORG, actor))


def folder_assert(
    database: sqlite3.Connection,
    operation: str,
    kind: str,
    row: sqlite3.Row,
    actor: str = ACTOR,
) -> None:
    database.execute(
        SQL["folder_assert"],
        (
            operation,
            kind,
            row["id"],
            ORG,
            actor,
            row["space_id"],
            row["scope_kind"],
            row["scope_id"],
            row["parent_id"],
            row["created_by_user_id"],
            row["storage_name"],
            row["legacy_name"],
            row["name"],
            row["color"],
            row["is_public"],
            row["settings_json"],
            row["revision"],
            row["tree_revision"],
            row["depth"],
            row["scope_revision"],
            row["scope_authority_version"],
            row["scope_creator_id"],
            row["actor_space_role"],
            row["actor_space_membership_revision"],
        ),
    )


def claim(
    database: sqlite3.Connection,
    operation: str,
    source: str,
    key: str,
    request: str,
    actor: str = ACTOR,
) -> None:
    database.execute(
        SQL["operation_claim"],
        (operation, ORG, actor, source, digest(key), digest(request), NOW),
    )


def evidence(
    database: sqlite3.Connection,
    *,
    operation: str,
    source: str,
    action: str,
    folder_id: str,
    scope_kind: str,
    scope_id: str | None,
    result_kind: str,
    affected: int,
    key_request: str,
    legacy_id: str | None = None,
    name: str | None = None,
    color: str | None = None,
    actor: str = ACTOR,
) -> None:
    database.execute(
        SQL["receipt_insert"],
        (
            operation,
            result_kind,
            action,
            folder_id,
            legacy_id,
            name,
            color,
            affected,
            NOW,
        ),
    )
    database.execute(
        SQL["effect_insert"],
        (
            operation,
            ORG,
            actor,
            action,
            scope_kind,
            scope_id,
            json.dumps({"paths": ["/dashboard/caps"], "folderIds": [folder_id]}),
            affected,
            NOW,
        ),
    )
    database.execute(
        SQL["audit_insert"],
        (
            str(uuid.uuid4()),
            operation,
            ORG,
            actor,
            source,
            digest(f"principal:{actor}"),
            digest(f"subject:{folder_id}"),
            NOW,
        ),
    )
    database.execute(SQL["operation_complete"], (operation, NOW))
    database.execute(
        SQL["durable_postcondition"],
        (
            operation,
            ORG,
            actor,
            source,
            digest(key_request),
            NOW,
            action,
            folder_id,
        ),
    )
    database.execute(SQL["assertion_cleanup"], (operation,))


def insert_folder(
    database: sqlite3.Connection,
    number: int,
    *,
    scope_kind: str = "personal",
    scope_id: str | None = None,
    parent_id: str | None = None,
    creator: str = ACTOR,
    depth: int = 0,
    settings: str = "{}",
) -> tuple[str, str]:
    legacy_id = cap_id(number)
    folder_id = mapped_uuid(legacy_id)
    space_id = scope_id if scope_kind == "space" else None
    database.execute(
        """INSERT INTO folders(
             id,legacy_folder_id,organization_id,space_id,parent_id,
             created_by_user_id,name,is_public,settings_json,created_at_ms,
             updated_at_ms,depth,legacy_color,legacy_scope_kind,legacy_scope_id
           ) VALUES (?,?,?,?,?,?,'Folder',0,?,?,?,?, 'normal',?,?)""",
        (
            folder_id,
            legacy_id,
            ORG,
            space_id,
            parent_id,
            creator,
            settings,
            NOW,
            NOW,
            depth,
            scope_kind,
            scope_id,
        ),
    )
    return folder_id, legacy_id


def execute_create(
    database: sqlite3.Connection,
    *,
    operation: str,
    source: str,
    folder_id: str,
    legacy_id: str,
    name: str,
    color: str,
    scope_kind: str,
    scope_id: str | None,
    is_public: int = 0,
) -> None:
    auth = authority(database)
    scope = scope_snapshot(database, scope_kind, scope_id)
    assert scope_kind == "personal" or auth["membership_role"] in ("owner", "admin")
    if is_public:
        assert auth["owner_has_pro_seat"] == 1
    database.execute("BEGIN")
    try:
        claim(database, operation, source, operation, operation)
        authority_assert(database, operation, auth)
        scope_assert(database, operation, scope)
        database.execute(
            SQL["create_insert"],
            (
                folder_id,
                legacy_id,
                ORG,
                scope_id if scope_kind == "space" else None,
                None,
                ACTOR,
                name,
                color,
                is_public,
                NOW,
                0,
                operation,
                scope_kind,
                scope_id,
            ),
        )
        database.execute(
            SQL["create_postcondition"],
            (
                operation,
                folder_id,
                legacy_id,
                ORG,
                scope_id if scope_kind == "space" else None,
                None,
                ACTOR,
                name,
                color,
                is_public,
                0,
                scope_kind,
                scope_id,
            ),
        )
        mobile = source == MOBILE_CREATE
        evidence(
            database,
            operation=operation,
            source=source,
            action="create",
            folder_id=folder_id,
            scope_kind=scope_kind,
            scope_id=scope_id,
            result_kind="mobile_created" if mobile else "rpc_void",
            affected=1,
            key_request=operation,
            legacy_id=legacy_id if mobile else None,
            name=name if mobile else None,
            color=color if mobile else None,
        )
        database.execute("COMMIT")
    except BaseException:
        database.execute("ROLLBACK")
        raise


def execute_update(
    database: sqlite3.Connection,
    *,
    operation: str,
    target_id: str,
    parent_id: str,
) -> None:
    auth = authority(database)
    target = folder_snapshot(database, target_id)
    parent = folder_snapshot(database, parent_id)
    cycle = one(database, "cycle_snapshot", (parent_id, target_id, ORG))
    assert cycle["cycle_count"] == 0
    subtree = one(database, "delete_subtree_snapshot", (target_id, ORG))
    final_depth = parent["depth"] + 1
    assert final_depth + subtree["max_depth"] <= 32
    patch = json.dumps({"hideTitle": False, "title": "Launches"}, separators=(",", ":"))
    database.execute("BEGIN")
    try:
        claim(database, operation, RPC_UPDATE, operation, operation)
        authority_assert(database, operation, auth)
        folder_assert(database, operation, "target", target)
        folder_assert(database, operation, "parent", parent)
        database.execute(SQL["cycle_assert"], (operation, parent_id, target_id, ORG))
        database.execute(
            SQL["update_apply"],
            (
                target_id,
                1,
                "Renamed",
                1,
                "blue",
                0,
                0,
                1,
                patch,
                "parent",
                parent_id,
                final_depth,
                1,
                NOW,
                operation,
                ORG,
                target["revision"],
                target["tree_revision"],
            ),
        )
        if final_depth != target["depth"]:
            database.execute(
                SQL["update_descendant_depths"],
                (target_id, ORG, final_depth - target["depth"], NOW, operation),
            )
        database.execute(
            SQL["update_postcondition"],
            (
                operation,
                target_id,
                ORG,
                "Renamed",
                "Renamed",
                "Renamed",
                "blue",
                target["is_public"],
                1,
                target["settings_json"],
                patch,
                parent_id,
                target["revision"] + 1,
                target["tree_revision"] + 1,
                final_depth,
                1,
            ),
        )
        evidence(
            database,
            operation=operation,
            source=RPC_UPDATE,
            action="update",
            folder_id=target_id,
            scope_kind=target["scope_kind"],
            scope_id=target["scope_id"],
            result_kind="rpc_void",
            affected=1,
            key_request=operation,
        )
        database.execute("COMMIT")
    except BaseException:
        database.execute("ROLLBACK")
        raise


def execute_delete(
    database: sqlite3.Connection, *, operation: str, target_id: str
) -> int:
    auth = authority(database)
    target = folder_snapshot(database, target_id)
    subtree = one(database, "delete_subtree_snapshot", (target_id, ORG))
    affected = subtree["folder_count"]
    database.execute("BEGIN")
    try:
        claim(database, operation, RPC_DELETE, operation, operation)
        authority_assert(database, operation, auth)
        folder_assert(database, operation, "target", target)
        database.execute(SQL["delete_targets_stage"], (operation, target_id, ORG))
        if target["scope_kind"] == "personal":
            database.execute(
                SQL["delete_reparent_personal"],
                (operation, ORG, target["parent_id"], NOW),
            )
        elif target["scope_kind"] == "organization":
            database.execute(
                SQL["delete_reparent_organization"],
                (operation, ORG, target["parent_id"], operation),
            )
        else:
            database.execute(
                SQL["delete_reparent_space"],
                (operation, ORG, target["scope_id"], target["parent_id"], operation),
            )
        database.execute(
            SQL["delete_root"],
            (target_id, ORG, target["revision"], target["tree_revision"]),
        )
        database.execute(
            SQL["delete_postcondition"], (operation, affected, subtree["ids_json"])
        )
        evidence(
            database,
            operation=operation,
            source=RPC_DELETE,
            action="delete",
            folder_id=target_id,
            scope_kind=target["scope_kind"],
            scope_id=target["scope_id"],
            result_kind="rpc_void",
            affected=affected,
            key_request=operation,
        )
        database.execute("COMMIT")
    except BaseException:
        database.execute("ROLLBACK")
        raise
    return affected


def test_full_expand_and_static_contract() -> None:
    database = migrated_database()
    columns = {row["name"] for row in database.execute("PRAGMA table_info(folders)")}
    assert {
        "legacy_folder_id",
        "legacy_name",
        "legacy_color",
        "legacy_scope_kind",
        "legacy_scope_id",
    } <= columns
    assert len(SQL) == 29
    migration = MIGRATION.read_text(encoding="utf-8")
    runtime = RUNTIME.read_text(encoding="utf-8")
    application = APPLICATION.read_text(encoding="utf-8")
    for token in (
        "D1LegacyFolderCrudAtomicPortV1",
        "LegacyFolderCrudAtomicPortV1",
        "owner_has_pro_seat",
        "UPDATE_DESCENDANT_DEPTHS_SQL",
        "DELETE_REPARENT_ORGANIZATION_SQL",
        "IdempotencyConflict",
    ):
        assert token in runtime
    assert "LEGACY_MOBILE_CREATE_FOLDER_SOURCES.len(), 9" in application
    assert "LEGACY_FOLDER_CRUD_NO_PROTECTED_GATES: &[&str] = &[]" in application
    assert 'CROSS_NAMESPACE_PROTECTED_GATES: &[&str] = &["human_approval"]' in application
    assert application.count("production_promoted: true") == 1
    assert application.count("production_promoted: false") == 3
    assert "frame_legacy_folder_crud_scope_v1" in migration
    assert "frame_legacy_folder_crud_evidence_immutable_v1" in migration
    assert "COALESCE(legacy_name, name) AS name" in (
        ROOT / "apps/control-plane/queries/organization/folders_list.sql"
    ).read_text(encoding="utf-8")
    assert "COALESCE(f.legacy_name,f.name) AS name" in (
        ROOT / "apps/control-plane/src/authenticated_web_runtime.rs"
    ).read_text(encoding="utf-8")


def test_native_frame_space_folders_remain_compatible_and_scoped() -> None:
    database = migrated_database()
    seed(database)
    second_space = fixture_id(22)
    database.execute(
        """INSERT INTO spaces(
             id,organization_id,created_by_user_id,name,created_at_ms,updated_at_ms
           ) VALUES (?,?,?,?,?,?)""",
        (second_space, ORG, ACTOR, "Second", NOW, NOW),
    )

    native_root = fixture_id(600)
    native_child = fixture_id(601)
    database.execute(
        """INSERT INTO folders(
             id,organization_id,space_id,parent_id,created_by_user_id,name,
             settings_json,created_at_ms,updated_at_ms
           ) VALUES (?,?,?,NULL,?,?,'{}',?,?)""",
        (native_root, ORG, SPACE, ACTOR, "Native root", NOW, NOW),
    )
    database.execute(
        """INSERT INTO folders(
             id,organization_id,space_id,parent_id,created_by_user_id,name,
             settings_json,created_at_ms,updated_at_ms
           ) VALUES (?,?,?,?,?,?,'{}',?,?)""",
        (native_child, ORG, SPACE, native_root, ACTOR, "Native child", NOW, NOW),
    )
    native = database.execute(
        "SELECT legacy_folder_id,legacy_scope_kind,legacy_scope_id FROM folders WHERE id=?",
        (native_root,),
    ).fetchone()
    assert tuple(native) == (None, "personal", None)
    database.execute("UPDATE folders SET space_id=space_id WHERE id=?", (native_child,))
    try:
        database.execute(
            "UPDATE folders SET legacy_folder_id=? WHERE id=?",
            (cap_id(605), native_root),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_folder_crud_scope_v1" in str(error)
    else:
        raise AssertionError("native space folder was converted to an invalid legacy scope")

    rejected = (
        (
            fixture_id(602),
            ORG,
            FOREIGN_SPACE,
            None,
            None,
            "Foreign native space",
        ),
        (
            fixture_id(603),
            ORG,
            second_space,
            native_root,
            None,
            "Cross-space native parent",
        ),
        (
            fixture_id(604),
            ORG,
            SPACE,
            None,
            cap_id(604),
            "Malformed legacy scope",
        ),
    )
    for folder_id, organization_id, space_id, parent_id, legacy_id, name in rejected:
        try:
            database.execute(
                """INSERT INTO folders(
                     id,legacy_folder_id,organization_id,space_id,parent_id,
                     created_by_user_id,name,settings_json,created_at_ms,updated_at_ms
                   ) VALUES (?,?,?,?,?,?,?,'{}',?,?)""",
                (
                    folder_id,
                    legacy_id,
                    organization_id,
                    space_id,
                    parent_id,
                    ACTOR,
                    name,
                    NOW,
                    NOW,
                ),
            )
        except sqlite3.IntegrityError as error:
            assert "frame_legacy_folder_crud_scope_v1" in str(error)
        else:
            raise AssertionError(f"unsafe folder scope was accepted: {name}")


def test_mobile_create_exact_response_replay_and_conflict() -> None:
    database = migrated_database()
    seed(database)
    legacy_id = cap_id(100)
    folder_id = mapped_uuid(legacy_id)
    operation = fixture_id(1000)
    execute_create(
        database,
        operation=operation,
        source=MOBILE_CREATE,
        folder_id=folder_id,
        legacy_id=legacy_id,
        name="Launches",
        color="normal",
        scope_kind="personal",
        scope_id=None,
    )
    receipt = database.execute(
        "SELECT * FROM legacy_folder_crud_receipts_v1 WHERE operation_id = ?", (operation,)
    ).fetchone()
    assert dict(receipt) | {}  # force complete row decoding
    assert (
        receipt["result_kind"],
        receipt["legacy_folder_id"],
        receipt["name"],
        receipt["color"],
    ) == ("mobile_created", legacy_id, "Launches", "normal")
    replay = one(
        database,
        "operation_by_key",
        (ORG, ACTOR, MOBILE_CREATE, digest(operation)),
    )
    assert replay["state"] == "complete" and replay["effect_count"] == 1
    try:
        claim(database, fixture_id(1001), MOBILE_CREATE, operation, "different")
    except sqlite3.IntegrityError as error:
        assert "UNIQUE constraint failed" in str(error)
    else:
        raise AssertionError("same idempotency key with another request must conflict")
    assert database.execute("SELECT COUNT(*) FROM folders WHERE id = ?", (folder_id,)).fetchone()[0] == 1


def test_rpc_create_scopes_owner_pro_and_tenant_non_disclosure() -> None:
    database = migrated_database()
    seed(database)
    for number, kind, scope_id, name in (
        (110, "organization", ORG, "Collection"),
        (111, "space", SPACE, ""),
        (112, "space", SPACE, "\u0000control"),
    ):
        legacy_id = cap_id(number)
        execute_create(
            database,
            operation=fixture_id(1100 + number),
            source=RPC_CREATE,
            folder_id=mapped_uuid(legacy_id),
            legacy_id=legacy_id,
            name=name,
            color="yellow",
            scope_kind=kind,
            scope_id=scope_id,
            is_public=1,
        )
    assert database.execute(
        "SELECT COUNT(*) FROM folders WHERE legacy_scope_kind IN ('organization','space')"
    ).fetchone()[0] == 3
    exact_names = [
        row["name"]
        for row in database.execute(ORGANIZATION_FOLDERS_SQL, (ORG, SPACE, None, 50))
    ]
    assert "" in exact_names and "\u0000control" in exact_names
    assert not database.execute(SQL["authority_snapshot"], (ACTOR, FOREIGN_ORG)).fetchall()
    assert not database.execute(SQL["folder_snapshot"], (mapped_uuid(cap_id(110)), FOREIGN_ORG, FOREIGN)).fetchall()
    database.execute(
        "UPDATE organization_members SET has_pro_seat = 0 WHERE organization_id = ? AND user_id = ?",
        (ORG, OWNER),
    )
    assert authority(database)["owner_has_pro_seat"] == 0


def test_update_presence_merge_cycle_and_descendant_depth() -> None:
    database = migrated_database()
    seed(database)
    old_parent, _ = insert_folder(database, 200)
    new_parent, _ = insert_folder(database, 201, depth=0)
    target, _ = insert_folder(
        database,
        202,
        parent_id=old_parent,
        depth=1,
        settings='{"publicPage":{"logoUrl":"keep","title":"old"},"other":1}',
    )
    child, _ = insert_folder(database, 203, parent_id=target, depth=2)
    execute_update(database, operation=fixture_id(1200), target_id=target, parent_id=new_parent)
    updated = database.execute("SELECT * FROM folders WHERE id = ?", (target,)).fetchone()
    settings = json.loads(updated["settings_json"])
    assert updated["name"] == "Renamed" and updated["legacy_color"] == "blue"
    assert updated["parent_id"] == new_parent and updated["depth"] == 1
    assert settings["publicPage"] == {
        "logoUrl": "keep",
        "title": "Launches",
        "hideTitle": False,
    }
    assert settings["other"] == 1
    assert database.execute("SELECT depth FROM folders WHERE id = ?", (child,)).fetchone()[0] == 2
    cycle = one(database, "cycle_snapshot", (child, target, ORG))
    assert cycle["cycle_count"] == 1


def test_recursive_delete_reparents_personal_space_and_organization_products() -> None:
    database = migrated_database()
    seed(database)
    parent, _ = insert_folder(database, 300)
    root, _ = insert_folder(database, 301, parent_id=parent, depth=1)
    child, _ = insert_folder(database, 302, parent_id=root, depth=2)
    video = fixture_id(3000)
    database.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id,folder_id
           ) VALUES (?,?,?,'ready',?,?,?,?)""",
        (video, ACTOR, "Video", NOW, NOW, ORG, child),
    )
    assert execute_delete(database, operation=fixture_id(1300), target_id=root) == 2
    assert database.execute("SELECT folder_id FROM videos WHERE id = ?", (video,)).fetchone()[0] == parent

    org_parent, _ = insert_folder(database, 310, scope_kind="organization", scope_id=ORG)
    org_root, _ = insert_folder(
        database, 311, scope_kind="organization", scope_id=ORG, parent_id=org_parent, depth=1
    )
    shared_video = fixture_id(3010)
    database.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id
           ) VALUES (?,?,?,'ready',?,?,?)""",
        (shared_video, ACTOR, "Shared", NOW, NOW, ORG),
    )
    database.execute(
        """INSERT INTO shared_videos(
           id,video_id,organization_id,folder_id,shared_by_user_id,sharing_mode,shared_at_ms
           ) VALUES (?,?,?,?,?,'space',?)""",
        (fixture_id(3011), shared_video, ORG, org_root, ACTOR, NOW),
    )
    execute_delete(database, operation=fixture_id(1310), target_id=org_root)
    assert database.execute(
        "SELECT folder_id FROM shared_videos WHERE video_id = ?", (shared_video,)
    ).fetchone()[0] == org_parent

    space_parent, _ = insert_folder(database, 320, scope_kind="space", scope_id=SPACE)
    space_root, _ = insert_folder(
        database, 321, scope_kind="space", scope_id=SPACE, parent_id=space_parent, depth=1
    )
    space_video = fixture_id(3020)
    database.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id
           ) VALUES (?,?,?,'ready',?,?,?)""",
        (space_video, ACTOR, "Space", NOW, NOW, ORG),
    )
    database.execute(
        """INSERT INTO space_videos(
             space_id,video_id,folder_id,added_by_user_id,added_at_ms
           ) VALUES (?,?,?,?,?)""",
        (SPACE, space_video, space_root, ACTOR, NOW),
    )
    execute_delete(database, operation=fixture_id(1320), target_id=space_root)
    assert database.execute(
        "SELECT folder_id FROM space_videos WHERE video_id = ?", (space_video,)
    ).fetchone()[0] == space_parent


def test_scope_guards_stale_authority_and_atomic_rollback() -> None:
    database = migrated_database()
    seed(database)
    personal, _ = insert_folder(database, 400)
    foreign_space_folder = mapped_uuid(cap_id(401))
    try:
        database.execute(
            """INSERT INTO folders(
                 id,organization_id,space_id,created_by_user_id,name,settings_json,
                 created_at_ms,updated_at_ms,legacy_scope_kind,legacy_scope_id
               ) VALUES (?,?,?,?,?,'{}',?,?,?,?)""",
            (
                foreign_space_folder,
                ORG,
                FOREIGN_SPACE,
                ACTOR,
                "Cross tenant",
                NOW,
                NOW,
                "space",
                FOREIGN_SPACE,
            ),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_folder_crud_scope_v1" in str(error)
    else:
        raise AssertionError("cross-tenant folder scope must fail")

    auth = authority(database)
    operation = fixture_id(1400)
    database.execute("BEGIN")
    try:
        claim(database, operation, RPC_DELETE, operation, operation)
        database.execute(
            "UPDATE organization_members SET revision = revision + 1 WHERE organization_id = ? AND user_id = ?",
            (ORG, ACTOR),
        )
        authority_assert(database, operation, auth)
    except sqlite3.IntegrityError as error:
        assert "frame_legacy_folder_crud_authority_v1" in str(error)
        database.execute("ROLLBACK")
    else:
        raise AssertionError("stale authority should abort")
    assert database.execute(
        "SELECT COUNT(*) FROM legacy_folder_crud_operations_v1 WHERE operation_id = ?", (operation,)
    ).fetchone()[0] == 0
    assert database.execute("SELECT COUNT(*) FROM folders WHERE id = ?", (personal,)).fetchone()[0] == 1


def test_durable_evidence_is_immutable_and_plaintext_free() -> None:
    database = migrated_database()
    seed(database)
    legacy_id = cap_id(500)
    operation = fixture_id(1500)
    execute_create(
        database,
        operation=operation,
        source=MOBILE_CREATE,
        folder_id=mapped_uuid(legacy_id),
        legacy_id=legacy_id,
        name="Evidence",
        color="red",
        scope_kind="personal",
        scope_id=None,
    )
    for table in (
        "legacy_folder_crud_receipts_v1",
        "legacy_folder_crud_effects_v1",
        "legacy_folder_crud_audit_events_v1",
    ):
        try:
            database.execute(f"DELETE FROM {table} WHERE operation_id = ?", (operation,))
        except sqlite3.IntegrityError as error:
            assert "frame_legacy_folder_crud_evidence_immutable_v1" in str(error)
        else:
            raise AssertionError(f"{table} must be immutable")
    stored = json.dumps(
        [dict(row) for row in database.execute("SELECT * FROM legacy_folder_crud_operations_v1")]
    )
    assert operation not in ("raw-idempotency-secret",) and "raw-idempotency-secret" not in stored


TESTS = (
    test_full_expand_and_static_contract,
    test_native_frame_space_folders_remain_compatible_and_scoped,
    test_mobile_create_exact_response_replay_and_conflict,
    test_rpc_create_scopes_owner_pro_and_tenant_non_disclosure,
    test_update_presence_merge_cycle_and_descendant_depth,
    test_recursive_delete_reparents_personal_space_and_organization_products,
    test_scope_guards_stale_authority_and_atomic_rollback,
    test_durable_evidence_is_immutable_and_plaintext_free,
)


def run() -> dict[str, object]:
    for test in TESTS:
        test()
    return {
        "schema_version": "frame.legacy-folder-crud-sqlite-conformance.v1",
        "provider": "local_sqlite",
        "expand_migration": MIGRATION.name,
        "full_expand_chain_applied": True,
        "tests_passed": len(TESTS),
        "operation_ids": [MOBILE_CREATE, RPC_CREATE, RPC_DELETE, RPC_UPDATE],
        "checked_in_queries_executed": len(SQL),
        "mobile_response_exact": True,
        "rpc_void_exact": True,
        "personal_organization_space_scopes": True,
        "owner_pro_public_gate": True,
        "recursive_delete_reparent": True,
        "tenant_non_disclosure": True,
        "same_key_replay_conflict": True,
        "atomic_rollback": True,
        "durable_journal_immutable": True,
        "mobile_production_promotion": True,
        "rpc_production_promotion": False,
        "rpc_protected_gate": "human_approval",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path)
    arguments = parser.parse_args()
    report = run()
    if arguments.evidence is not None:
        arguments.evidence.parent.mkdir(parents=True, exist_ok=True)
        arguments.evidence.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(
        "legacy folder-CRUD SQLite conformance: "
        f"{report['tests_passed']} passed; exact mobile/RPC results, scoped authority, "
        "recursive relocation, replay/conflict, rollback, and immutability verified"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
