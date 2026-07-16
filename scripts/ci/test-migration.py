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
    print("ETL interruption, resume, quarantine, replay, and mismatch checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
