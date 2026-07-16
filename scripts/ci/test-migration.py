#!/usr/bin/env python3
"""Exercise ETL transforms, interruption/resume, quarantine, and reconciliation."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sqlite3
import sys
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("frame_migration", ROOT / "scripts" / "migration.py")
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load migration module")
MIGRATION = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MIGRATION
SPEC.loader.exec_module(MIGRATION)

D1_MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
DIRTY_ORGANIZATION = "018f47a6-7b1c-7f55-8f39-8f8a8690e201"
OWNER_A = "018f47a6-7b1c-7f55-8f39-8f8a8690a201"
OWNER_B = "018f47a6-7b1c-7f55-8f39-8f8a8690a202"
OWNER_C = "018f47a6-7b1c-7f55-8f39-8f8a8690a203"
DIRTY_SPACE = "018f47a6-7b1c-7f55-8f39-8f8a8690b201"
ROOT_FOLDER = "018f47a6-7b1c-7f55-8f39-8f8a8690c201"
CHILD_FOLDER = "018f47a6-7b1c-7f55-8f39-8f8a8690c202"
GRANDCHILD_FOLDER = "018f47a6-7b1c-7f55-8f39-8f8a8690c203"
CYCLE_A = "018f47a6-7b1c-7f55-8f39-8f8a8690c204"
CYCLE_B = "018f47a6-7b1c-7f55-8f39-8f8a8690c205"
FIXTURE_OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a8690d201"
NORMALIZE_OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a8690d202"


def initialize(path: pathlib.Path, with_rows: bool) -> None:
    database = sqlite3.connect(path)
    database.executescript(
        """
        PRAGMA foreign_keys = ON;
        CREATE TABLE parents (
          id TEXT PRIMARY KEY,
          enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
          created_at_ms INTEGER NOT NULL,
          settings_json TEXT NOT NULL
        );
        CREATE TABLE children (
          id TEXT PRIMARY KEY,
          parent_id TEXT NOT NULL REFERENCES parents(id),
          amount INTEGER NOT NULL
        );
        """
    )
    if with_rows:
        database.executemany(
            "INSERT INTO parents VALUES (?, ?, ?, ?)",
            [
                ("p1", 1, 1_700_000_000_000, '{"z":2,"a":1}'),
                ("p2", 0, 1_700_000_000_001, "{}"),
                ("p3", 1, -1, "{}"),
            ],
        )
        database.executemany(
            "INSERT INTO children VALUES (?, ?, ?)",
            [("c1", "p1", 10), ("c2", "p2", 20)],
        )
    database.commit()
    database.close()


def write_plan(path: pathlib.Path) -> None:
    plan = {
        "schema_version": 1,
        "run_id": "ci-rehearsal-0001",
        "source_schema": "fixture-v1",
        "target_migration": "fixture-v1",
        "code_revision": "working-tree",
        "tables": [
            {
                "name": "parents",
                "columns": ["id", "enabled", "created_at_ms", "settings_json"],
                "primary_key": ["id"],
                "transforms": {
                    "enabled": "boolean",
                    "created_at_ms": "timestamp_ms",
                    "settings_json": "json",
                },
            },
            {
                "name": "children",
                "columns": ["id", "parent_id", "amount"],
                "primary_key": ["id"],
                "transforms": {"amount": "wire_integer"},
            },
        ],
    }
    path.write_text(json.dumps(plan), encoding="utf-8")


def d1_migration_files() -> list[pathlib.Path]:
    return sorted(D1_MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql"))


def insert_dirty_organization_fixture(database: sqlite3.Connection) -> None:
    now = 1_700_300_000_000
    for user_id, email in (
        (OWNER_A, "owner-a@migration.invalid"),
        (OWNER_B, "owner-b@migration.invalid"),
        (OWNER_C, "owner-c@migration.invalid"),
    ):
        database.execute(
            "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) "
            "VALUES (?, ?, NULL, ?, ?)",
            (user_id, email, now, now),
        )
        database.execute(
            "INSERT INTO auth_identities_v2("
            "user_id,identity_revision,session_version,created_at_ms,updated_at_ms,"
            "revision,last_operation_id) VALUES (?,1,0,?,?,0,?)",
            (user_id, now, now, FIXTURE_OPERATION),
        )
    database.execute(
        "INSERT INTO organizations("
        "id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,"
        "tombstoned_at_ms,revision) VALUES (?,?,'Dirty legacy','active','{}',?,?,NULL,0)",
        (DIRTY_ORGANIZATION, OWNER_A, now, now),
    )
    for owner_id in (OWNER_A, OWNER_B):
        database.execute(
            "INSERT INTO organization_members("
            "organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms,revision) "
            "VALUES (?,?,'owner','active',0,?,?,0)",
            (DIRTY_ORGANIZATION, owner_id, now, now),
        )
    database.execute(
        "INSERT INTO spaces("
        "id,organization_id,created_by_user_id,name,is_primary,is_public,settings_json,"
        "created_at_ms,updated_at_ms,deleted_at_ms,revision) "
        "VALUES (?,?,?,'Legacy',1,0,'{}',?,?,NULL,0)",
        (DIRTY_SPACE, DIRTY_ORGANIZATION, OWNER_A, now, now),
    )
    folders = (
        (ROOT_FOLDER, None, "Root"),
        (CHILD_FOLDER, ROOT_FOLDER, "Child"),
        (GRANDCHILD_FOLDER, CHILD_FOLDER, "Grandchild"),
        (CYCLE_A, None, "Cycle A"),
        (CYCLE_B, None, "Cycle B"),
    )
    for folder_id, parent_id, name in folders:
        database.execute(
            "INSERT INTO folders("
            "id,organization_id,space_id,parent_id,created_by_user_id,name,is_public,"
            "settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision) "
            "VALUES (?,?,?,?,?,?,0,'{}',?,?,NULL,0)",
            (
                folder_id,
                DIRTY_ORGANIZATION,
                DIRTY_SPACE,
                parent_id,
                OWNER_A,
                name,
                now,
                now,
            ),
        )
    database.execute("UPDATE folders SET parent_id=? WHERE id=?", (CYCLE_B, CYCLE_A))
    database.execute("UPDATE folders SET parent_id=? WHERE id=?", (CYCLE_A, CYCLE_B))
    database.commit()


def rebuild_reviewed_folder_tree(database: sqlite3.Connection) -> None:
    database.execute(
        "UPDATE folders SET parent_id=NULL, revision=revision+1, tree_revision=tree_revision+1, "
        "last_operation_id=? WHERE id=? AND organization_id=? AND space_id=?",
        (NORMALIZE_OPERATION, CYCLE_A, DIRTY_ORGANIZATION, DIRTY_SPACE),
    )
    database.execute(
        "DELETE FROM organization_folder_closure_v1 WHERE organization_id=? AND space_id=?",
        (DIRTY_ORGANIZATION, DIRTY_SPACE),
    )
    database.execute(
        "INSERT INTO organization_folder_closure_v1("
        "organization_id,space_id,ancestor_id,descendant_id,distance) "
        "SELECT organization_id,space_id,id,id,0 FROM folders "
        "WHERE organization_id=? AND space_id=?",
        (DIRTY_ORGANIZATION, DIRTY_SPACE),
    )
    database.execute(
        """
        WITH RECURSIVE reviewed_tree(
          organization_id, space_id, ancestor_id, descendant_id, distance
        ) AS (
          SELECT organization_id, space_id, id, id, 0
          FROM folders
          WHERE organization_id=?1 AND space_id=?2
          UNION ALL
          SELECT reviewed_tree.organization_id,
                 reviewed_tree.space_id,
                 reviewed_tree.ancestor_id,
                 child.id,
                 reviewed_tree.distance + 1
          FROM reviewed_tree
          JOIN folders child
            ON child.parent_id = reviewed_tree.descendant_id
           AND child.organization_id = reviewed_tree.organization_id
           AND child.space_id = reviewed_tree.space_id
          WHERE reviewed_tree.distance < 32
        )
        INSERT OR IGNORE INTO organization_folder_closure_v1(
          organization_id,space_id,ancestor_id,descendant_id,distance
        )
        SELECT organization_id,space_id,ancestor_id,descendant_id,distance
        FROM reviewed_tree
        """,
        (DIRTY_ORGANIZATION, DIRTY_SPACE),
    )
    database.execute(
        "UPDATE organizations SET revision=revision+1,authority_version=authority_version+1,"
        "last_operation_id=? WHERE id=?",
        (NORMALIZE_OPERATION, DIRTY_ORGANIZATION),
    )
    database.execute(
        "UPDATE auth_identities_v2 SET session_version=session_version+1,"
        "revision=revision+1,last_operation_id=? WHERE user_id IN ("
        "SELECT user_id FROM organization_members "
        "WHERE organization_id=? AND state='active')",
        (NORMALIZE_OPERATION, DIRTY_ORGANIZATION),
    )
    database.execute(
        "DELETE FROM auth_session_mutation_grants_v2 WHERE user_id IN ("
        "SELECT user_id FROM auth_identities_v2 WHERE last_operation_id=?)",
        (NORMALIZE_OPERATION,),
    )
    database.execute(
        "UPDATE folders SET depth=0,tree_revision=tree_revision+1,last_operation_id=? "
        "WHERE organization_id=? AND space_id=? AND deleted_at_ms IS NULL",
        (NORMALIZE_OPERATION, DIRTY_ORGANIZATION, DIRTY_SPACE),
    )
    database.execute(
        """
        WITH RECURSIVE rooted(organization_id,space_id,folder_id,depth) AS (
          SELECT organization_id,space_id,id,0
          FROM folders
          WHERE organization_id=?1 AND space_id=?2
            AND parent_id IS NULL AND deleted_at_ms IS NULL
          UNION ALL
          SELECT child.organization_id,child.space_id,child.id,rooted.depth+1
          FROM rooted JOIN folders child
            ON child.parent_id=rooted.folder_id
           AND child.organization_id=rooted.organization_id
           AND child.space_id=rooted.space_id
           AND child.deleted_at_ms IS NULL
          WHERE rooted.depth < 32
        )
        UPDATE folders
        SET depth=(SELECT depth FROM rooted WHERE folder_id=folders.id)
        WHERE id IN (SELECT folder_id FROM rooted)
        """,
        (DIRTY_ORGANIZATION, DIRTY_SPACE),
    )


def exercise_dirty_organization_upgrade() -> None:
    files = d1_migration_files()
    assert len(files) >= 10
    database = sqlite3.connect(":memory:")
    database.execute("PRAGMA foreign_keys = ON")
    try:
        for path in files[:9]:
            database.executescript(path.read_text(encoding="utf-8"))
        insert_dirty_organization_fixture(database)
        database.executescript(files[9].read_text(encoding="utf-8"))

        owner_state = database.execute(
            "SELECT active_owner_count,pointer_owner_count "
            "FROM organization_owner_integrity_v1 WHERE organization_id=?",
            (DIRTY_ORGANIZATION,),
        ).fetchone()
        assert owner_state == (2, 1)
        assert database.execute(
            "SELECT depth FROM folders WHERE id=?", (GRANDCHILD_FOLDER,)
        ).fetchone() == (2,)

        cycle_rows = database.execute(
            "SELECT COUNT(*) FROM organization_folder_closure_v1 "
            "WHERE organization_id=? AND space_id=? "
            "AND ancestor_id=descendant_id AND distance<>0",
            (DIRTY_ORGANIZATION, DIRTY_SPACE),
        ).fetchone()
        assert cycle_rows is not None and cycle_rows[0] >= 2
        graph_sql = (
            ROOT
            / "apps"
            / "control-plane"
            / "queries"
            / "organization"
            / "graph_audit_folders.sql"
        ).read_text(encoding="utf-8")
        findings = database.execute(graph_sql, (DIRTY_ORGANIZATION, 100)).fetchall()
        cycle_findings = [row[1] for row in findings if row[0] == "folder_cycle"]
        assert set(cycle_findings) >= {
            CYCLE_A,
            CYCLE_B,
        }
        assert len(cycle_findings) == len(set(cycle_findings))

        try:
            database.execute(
                "INSERT INTO organization_members("
                "organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms,revision) "
                "VALUES (?,?,'owner','active',0,1,1,0)",
                (DIRTY_ORGANIZATION, OWNER_C),
            )
        except sqlite3.IntegrityError as error:
            assert "frame_organization_cas_conflict_v1" in str(error)
            database.rollback()
        else:
            raise AssertionError("dirty organization accepted another active owner")

        database.execute(
            "UPDATE organization_members SET role='admin',revision=revision+1,"
            "authority_version=authority_version+1,last_operation_id=? "
            "WHERE organization_id=? AND user_id=? AND role='owner' AND state='active'",
            (NORMALIZE_OPERATION, DIRTY_ORGANIZATION, OWNER_A),
        )
        database.execute(
            "UPDATE organizations SET owner_id=?,revision=revision+1,"
            "authority_version=authority_version+1,last_operation_id=? WHERE id=?",
            (OWNER_B, NORMALIZE_OPERATION, DIRTY_ORGANIZATION),
        )
        database.execute(
            "UPDATE auth_identities_v2 SET session_version=session_version+1,"
            "revision=revision+1,last_operation_id=? WHERE user_id IN (?,?)",
            (NORMALIZE_OPERATION, OWNER_A, OWNER_B),
        )
        database.execute(
            "DELETE FROM auth_session_mutation_grants_v2 WHERE user_id IN (?,?)",
            (OWNER_A, OWNER_B),
        )
        rebuild_reviewed_folder_tree(database)
        database.commit()

        assert database.execute(
            "SELECT active_owner_count,pointer_owner_count "
            "FROM organization_owner_integrity_v1 WHERE organization_id=?",
            (DIRTY_ORGANIZATION,),
        ).fetchone() == (1, 1)
        assert database.execute(
            "SELECT COUNT(*) FROM organization_folder_closure_v1 "
            "WHERE organization_id=? AND space_id=? "
            "AND ancestor_id=descendant_id AND distance<>0",
            (DIRTY_ORGANIZATION, DIRTY_SPACE),
        ).fetchone() == (0,)
        assert database.execute(
            "SELECT depth FROM folders WHERE id=?", (CYCLE_B,)
        ).fetchone() == (1,)
        assert database.execute("PRAGMA foreign_key_check").fetchall() == []

        # The expand-phase guards must still permit the repository's ordered
        # demote/pointer/promote transfer sequence on a clean organization.
        database.execute("SAVEPOINT clean_transfer")
        database.execute(
            "UPDATE organization_members SET role='admin' "
            "WHERE organization_id=? AND user_id=?",
            (DIRTY_ORGANIZATION, OWNER_B),
        )
        database.execute(
            "UPDATE organizations SET owner_id=? WHERE id=?",
            (OWNER_A, DIRTY_ORGANIZATION),
        )
        database.execute(
            "UPDATE organization_members SET role='owner' "
            "WHERE organization_id=? AND user_id=?",
            (DIRTY_ORGANIZATION, OWNER_A),
        )
        assert database.execute(
            "SELECT active_owner_count,pointer_owner_count "
            "FROM organization_owner_integrity_v1 WHERE organization_id=?",
            (DIRTY_ORGANIZATION,),
        ).fetchone() == (1, 1)
        database.execute("ROLLBACK TO clean_transfer")
        database.execute("RELEASE clean_transfer")

        # This models the later contract migration. It must be delayed until the
        # reviewed cleanup proves every organization is clean.
        database.execute(
            "CREATE UNIQUE INDEX organization_members_one_active_owner_contract_idx "
            "ON organization_members(organization_id) "
            "WHERE role='owner' AND state='active'"
        )
    finally:
        database.close()


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-etl-") as directory:
        root = pathlib.Path(directory)
        source = root / "source.sqlite"
        target = root / "target.sqlite"
        plan_path = root / "plan.json"
        bundle = root / "bundle"
        report_path = root / "report.json"
        initialize(source, with_rows=True)
        initialize(target, with_rows=False)
        write_plan(plan_path)
        plan = MIGRATION.load_plan(plan_path)
        manifest = MIGRATION.export_bundle(source, plan, bundle, chunk_rows=1)
        assert manifest["row_count"] == 4
        assert manifest["reject_count"] == 1
        assert (bundle.stat().st_mode & 0o077) == 0
        assert ((bundle / "manifest.json").stat().st_mode & 0o077) == 0
        quarantine = (bundle / "quarantine.ndjson").read_text(encoding="utf-8")
        assert "-1" not in quarantine
        assert "invalid_timestamp" in quarantine

        try:
            MIGRATION.import_bundle(target, bundle, dry_run=False, interrupt_after=2)
        except InterruptedError:
            pass
        else:
            raise AssertionError("injected interruption did not occur")
        applied, skipped = MIGRATION.import_bundle(
            target, bundle, dry_run=False, interrupt_after=None
        )
        assert applied == 2
        assert skipped == 2
        replay_applied, replay_skipped = MIGRATION.import_bundle(
            target, bundle, dry_run=False, interrupt_after=None
        )
        assert replay_applied == 0
        assert replay_skipped == 4

        report = MIGRATION.reconcile_bundle(target, bundle)
        assert report["clean"] is True
        MIGRATION.write_private(report_path, f"{MIGRATION.canonical(report)}\n")
        target_database = sqlite3.connect(target)
        target_database.execute("UPDATE children SET amount = 99 WHERE id = 'c1'")
        target_database.commit()
        target_database.close()
        mismatch = MIGRATION.reconcile_bundle(target, bundle)
        assert mismatch["clean"] is False
        assert mismatch["unexplained_mismatches"] == 1
        assert "c1" not in MIGRATION.canonical(mismatch)
    exercise_dirty_organization_upgrade()
    print(
        "ETL interruption/resume plus dirty organization owner/folder migration checks passed"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
