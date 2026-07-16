#!/usr/bin/env python3
"""Apply every D1/SQLite migration and enforce expand-first invariants."""

from __future__ import annotations

import pathlib
import re
import sqlite3
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
NAME = re.compile(r"^(\d{4})_[a-z0-9_]+\.sql$")
DESTRUCTIVE = re.compile(
    r"\b(DROP\s+(?:TABLE|COLUMN|INDEX)|TRUNCATE|VACUUM|DELETE\s+FROM)\b",
    re.IGNORECASE,
)
REQUIRED_TABLES = {
    "auth_api_keys",
    "authority_state",
    "comments",
    "command_idempotency",
    "developer_apps",
    "developer_credit_transactions",
    "etl_checkpoints",
    "folders",
    "identity_accounts",
    "media_jobs",
    "multipart_upload_parts",
    "multipart_uploads",
    "object_deletion_jobs",
    "object_legal_holds",
    "object_manifests",
    "organizations",
    "organization_members",
    "outbox_events",
    "sessions",
    "shared_videos",
    "spaces",
    "storage_objects",
    "users",
    "video_edits",
    "video_uploads",
    "videos",
}


def discover() -> list[pathlib.Path]:
    files = sorted(MIGRATIONS.glob("*.sql"))
    numbers: list[int] = []
    for path in files:
        match = NAME.fullmatch(path.name)
        if not match:
            raise ValueError(f"invalid migration filename: {path.name}")
        numbers.append(int(match.group(1)))
    if not files:
        raise ValueError("no migrations found")
    expected = list(range(numbers[0], numbers[-1] + 1))
    if numbers != expected:
        raise ValueError(f"migration sequence is not contiguous: {numbers}")
    return files


def apply(files: list[pathlib.Path]) -> sqlite3.Connection:
    database = sqlite3.connect(":memory:")
    database.execute("PRAGMA foreign_keys = ON")
    for path in files:
        sql = path.read_text(encoding="utf-8")
        if DESTRUCTIVE.search(sql):
            raise ValueError(f"destructive statement in expand migration: {path.name}")
        try:
            database.executescript(sql)
        except sqlite3.Error as error:
            raise ValueError(f"{path.name} failed: {error}") from error
        violations = database.execute("PRAGMA foreign_key_check").fetchall()
        if violations:
            raise ValueError(f"{path.name} introduced foreign-key violations")
    return database


def main() -> int:
    try:
        files = discover()
        database = apply(files)
        tables = {
            row[0]
            for row in database.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
            )
        }
        missing = sorted(REQUIRED_TABLES - tables)
        if missing:
            raise ValueError(f"required tables missing after migration: {', '.join(missing)}")

        # The only released baseline before this change is migration 0001. Apply
        # it independently, then prove the complete ordered upgrade path.
        baseline = apply(files[:1])
        for path in files[1:]:
            baseline.executescript(path.read_text(encoding="utf-8"))
        violations = baseline.execute("PRAGMA foreign_key_check").fetchall()
        if violations:
            raise ValueError("0001 upgrade path has foreign-key violations")
    except (OSError, ValueError, sqlite3.Error) as error:
        print(f"migration validation failed: {error}", file=sys.stderr)
        return 1

    print(f"validated {len(files)} ordered expand-first migrations and 0001 upgrade path")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
