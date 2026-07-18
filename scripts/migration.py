#!/usr/bin/env python3
"""Deterministic, resumable SQLite rehearsal pipeline for the MySQL-to-D1 ETL contract.

The production MySQL reader emits the same versioned bundle format. This tool deliberately
ships only the credential-free SQLite reader used for CI, restore drills, and local rehearsals.
It never logs row values and creates bundle files with owner-only permissions.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import sqlite3
import sys
import tempfile
from dataclasses import dataclass
from typing import Any, Iterable, Iterator


SCHEMA_VERSION = 1
MAX_WIRE_INTEGER = 9_007_199_254_740_991
IDENTIFIER = re.compile(r"^[a-z][a-z0-9_]{0,62}$")
SAFE_RUN_ID = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9_.:-]{7,127}$")
TRANSFORMS = {"identity", "boolean", "json", "timestamp_ms", "wire_integer"}


class MigrationError(Exception):
    """A safe operator-facing failure that contains no source row values."""


@dataclass(frozen=True)
class TablePlan:
    name: str
    columns: tuple[str, ...]
    primary_key: tuple[str, ...]
    transforms: dict[str, str]


@dataclass(frozen=True)
class MigrationPlan:
    run_id: str
    source_schema: str
    target_migration: str
    code_revision: str
    tables: tuple[TablePlan, ...]


def private_directory(path: pathlib.Path) -> None:
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.chmod(0o700)


def write_private(path: pathlib.Path, data: str) -> None:
    private_directory(path.parent)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary_path = pathlib.Path(temporary)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        temporary_path.replace(path)
        path.chmod(0o600)
    except BaseException:
        temporary_path.unlink(missing_ok=True)
        raise


def canonical(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def digest_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def digest_row(row: dict[str, Any]) -> str:
    return digest_bytes(canonical(row).encode("utf-8"))


def quote_identifier(value: str) -> str:
    if not IDENTIFIER.fullmatch(value):
        raise MigrationError("plan contains an invalid SQL identifier")
    return f'"{value}"'


def safe_label(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value or len(value) > 128:
        raise MigrationError(f"plan {label} is invalid")
    if any(character.isspace() for character in value):
        raise MigrationError(f"plan {label} is invalid")
    return value


def load_plan(path: pathlib.Path) -> MigrationPlan:
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise MigrationError("migration plan is unreadable or invalid JSON") from error
    if raw.get("schema_version") != SCHEMA_VERSION:
        raise MigrationError("migration plan schema version is unsupported")
    run_id = raw.get("run_id")
    if not isinstance(run_id, str) or not SAFE_RUN_ID.fullmatch(run_id):
        raise MigrationError("migration plan run_id is invalid")

    tables: list[TablePlan] = []
    seen: set[str] = set()
    for raw_table in raw.get("tables", []):
        name = raw_table.get("name")
        quote_identifier(name)
        if name in seen:
            raise MigrationError("migration plan repeats a table")
        seen.add(name)
        columns = tuple(raw_table.get("columns", []))
        primary_key = tuple(raw_table.get("primary_key", []))
        if not columns or not primary_key or not set(primary_key).issubset(columns):
            raise MigrationError("migration table columns or primary key are invalid")
        for column in columns:
            quote_identifier(column)
        transforms = raw_table.get("transforms", {})
        if not isinstance(transforms, dict) or not set(transforms).issubset(columns):
            raise MigrationError("migration transforms reference an unknown column")
        if not set(transforms.values()).issubset(TRANSFORMS):
            raise MigrationError("migration plan names an unsupported transform")
        tables.append(TablePlan(name, columns, primary_key, transforms))
    if not tables:
        raise MigrationError("migration plan has no tables")
    return MigrationPlan(
        run_id=run_id,
        source_schema=safe_label(raw.get("source_schema"), "source_schema"),
        target_migration=safe_label(raw.get("target_migration"), "target_migration"),
        code_revision=safe_label(raw.get("code_revision"), "code_revision"),
        tables=tuple(tables),
    )


def transform_value(kind: str, value: Any) -> Any:
    if value is None or kind == "identity":
        return value
    if kind == "boolean":
        if value in (0, False, "0"):
            return 0
        if value in (1, True, "1"):
            return 1
        raise ValueError("invalid_boolean")
    if kind == "json":
        parsed = json.loads(value) if isinstance(value, str) else value
        return canonical(parsed)
    if kind == "timestamp_ms":
        integer = int(value)
        if not 0 <= integer <= 253_402_300_799_999:
            raise ValueError("invalid_timestamp")
        return integer
    if kind == "wire_integer":
        integer = int(value)
        if not -MAX_WIRE_INTEGER <= integer <= MAX_WIRE_INTEGER:
            raise ValueError("unsafe_wire_integer")
        return integer
    raise ValueError("unsupported_transform")


def transform_row(table: TablePlan, row: sqlite3.Row) -> dict[str, Any]:
    transformed: dict[str, Any] = {}
    for column in table.columns:
        transformed[column] = transform_value(table.transforms.get(column, "identity"), row[column])
    if any(transformed[column] is None for column in table.primary_key):
        raise ValueError("missing_primary_key")
    return transformed


def chunked(rows: Iterable[dict[str, Any]], size: int) -> Iterator[list[dict[str, Any]]]:
    chunk: list[dict[str, Any]] = []
    for row in rows:
        chunk.append(row)
        if len(chunk) == size:
            yield chunk
            chunk = []
    if chunk:
        yield chunk


def open_read_only(path: pathlib.Path) -> sqlite3.Connection:
    if not path.is_file():
        raise MigrationError("source database does not exist")
    connection = sqlite3.connect(f"file:{path.resolve()}?mode=ro", uri=True)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA query_only = ON")
    return connection


def table_rows(connection: sqlite3.Connection, table: TablePlan) -> Iterator[sqlite3.Row]:
    columns = ", ".join(quote_identifier(column) for column in table.columns)
    order = ", ".join(quote_identifier(column) for column in table.primary_key)
    query = f"SELECT {columns} FROM {quote_identifier(table.name)} ORDER BY {order}"
    yield from connection.execute(query)


def export_bundle(
    source: pathlib.Path,
    plan: MigrationPlan,
    bundle: pathlib.Path,
    chunk_rows: int,
) -> dict[str, Any]:
    if not 1 <= chunk_rows <= 100_000:
        raise MigrationError("chunk size must be between 1 and 100000")
    private_directory(bundle)
    connection = open_read_only(source)
    manifest_tables: list[dict[str, Any]] = []
    rejects: list[dict[str, Any]] = []
    try:
        connection.execute("BEGIN")
        for table in plan.tables:
            valid_rows: list[dict[str, Any]] = []
            for ordinal, row in enumerate(table_rows(connection, table), start=1):
                try:
                    valid_rows.append(transform_row(table, row))
                except (TypeError, ValueError, json.JSONDecodeError) as error:
                    rejects.append(
                        {
                            "table": table.name,
                            "ordinal": ordinal,
                            "reason_code": str(error).split(":", maxsplit=1)[0],
                        }
                    )
            chunks: list[dict[str, Any]] = []
            for index, rows in enumerate(chunked(valid_rows, chunk_rows), start=1):
                relative = pathlib.Path("chunks") / table.name / f"{index:06d}.ndjson"
                payload = "".join(f"{canonical(row)}\n" for row in rows)
                write_private(bundle / relative, payload)
                chunks.append(
                    {
                        "path": relative.as_posix(),
                        "rows": len(rows),
                        "sha256": digest_bytes(payload.encode("utf-8")),
                    }
                )
            manifest_tables.append(
                {
                    "name": table.name,
                    "columns": list(table.columns),
                    "primary_key": list(table.primary_key),
                    "row_count": len(valid_rows),
                    "chunks": chunks,
                }
            )
        connection.rollback()
    finally:
        connection.close()

    if rejects:
        payload = "".join(f"{canonical(reject)}\n" for reject in rejects)
        write_private(bundle / "quarantine.ndjson", payload)
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "run_id": plan.run_id,
        "source_schema": plan.source_schema,
        "target_migration": plan.target_migration,
        "code_revision": plan.code_revision,
        "table_count": len(manifest_tables),
        "row_count": sum(table["row_count"] for table in manifest_tables),
        "reject_count": len(rejects),
        "tables": manifest_tables,
    }
    write_private(bundle / "manifest.json", f"{canonical(manifest)}\n")
    return manifest


def load_manifest(bundle: pathlib.Path) -> dict[str, Any]:
    try:
        manifest = json.loads((bundle / "manifest.json").read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise MigrationError("bundle manifest is unreadable or invalid") from error
    if manifest.get("schema_version") != SCHEMA_VERSION:
        raise MigrationError("bundle schema version is unsupported")
    return manifest


def read_chunk(bundle: pathlib.Path, chunk: dict[str, Any]) -> list[dict[str, Any]]:
    relative = pathlib.PurePosixPath(chunk["path"])
    if relative.is_absolute() or ".." in relative.parts:
        raise MigrationError("bundle contains an unsafe chunk path")
    path = bundle.joinpath(*relative.parts)
    payload = path.read_bytes()
    if digest_bytes(payload) != chunk["sha256"]:
        raise MigrationError("bundle chunk checksum mismatch")
    try:
        rows = [json.loads(line) for line in payload.decode("utf-8").splitlines()]
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise MigrationError("bundle chunk is invalid") from error
    if len(rows) != chunk["rows"]:
        raise MigrationError("bundle chunk row count mismatch")
    return rows


def checkpoint_path(bundle: pathlib.Path, target: pathlib.Path) -> pathlib.Path:
    target_digest = digest_bytes(str(target.resolve()).encode("utf-8"))[:16]
    return bundle / "checkpoints" / f"target-{target_digest}.json"


def load_checkpoint(path: pathlib.Path, run_id: str) -> dict[str, Any]:
    if not path.exists():
        return {"schema_version": SCHEMA_VERSION, "run_id": run_id, "completed": []}
    try:
        checkpoint = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise MigrationError("target checkpoint is invalid") from error
    if checkpoint.get("schema_version") != SCHEMA_VERSION or checkpoint.get("run_id") != run_id:
        raise MigrationError("target checkpoint belongs to another run")
    return checkpoint


def upsert_chunk(
    connection: sqlite3.Connection,
    table: dict[str, Any],
    rows: list[dict[str, Any]],
) -> None:
    columns = table["columns"]
    primary_key = table["primary_key"]
    for identifier in [table["name"], *columns, *primary_key]:
        quote_identifier(identifier)
    column_sql = ", ".join(quote_identifier(column) for column in columns)
    placeholders = ", ".join("?" for _ in columns)
    conflict = ", ".join(quote_identifier(column) for column in primary_key)
    insert = (
        f"INSERT INTO {quote_identifier(table['name'])} ({column_sql}) "
        f"VALUES ({placeholders}) ON CONFLICT ({conflict}) DO NOTHING"
    )
    select_where = " AND ".join(f"{quote_identifier(column)} IS ?" for column in primary_key)
    select = f"SELECT {column_sql} FROM {quote_identifier(table['name'])} WHERE {select_where}"
    for row in rows:
        if set(row) != set(columns):
            raise MigrationError("bundle row shape differs from manifest")
        connection.execute(insert, tuple(row[column] for column in columns))
        key = tuple(row[column] for column in primary_key)
        target = connection.execute(select, key).fetchone()
        if target is None:
            raise MigrationError("target row is missing after insert")
        target_row = {column: target[index] for index, column in enumerate(columns)}
        if digest_row(target_row) != digest_row(row):
            raise MigrationError("target primary key already contains different data")


def import_bundle(
    target: pathlib.Path,
    bundle: pathlib.Path,
    dry_run: bool,
    interrupt_after: int | None,
) -> tuple[int, int]:
    manifest = load_manifest(bundle)
    checkpoint_file = checkpoint_path(bundle, target)
    checkpoint = load_checkpoint(checkpoint_file, manifest["run_id"])
    completed = set(checkpoint["completed"])
    applied = 0
    skipped = 0
    connection = sqlite3.connect(target)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA foreign_keys = ON")
    try:
        for table in manifest["tables"]:
            for chunk in table["chunks"]:
                chunk_id = f"{table['name']}:{chunk['sha256']}"
                if chunk_id in completed:
                    skipped += 1
                    continue
                rows = read_chunk(bundle, chunk)
                connection.execute("BEGIN IMMEDIATE")
                try:
                    upsert_chunk(connection, table, rows)
                    if dry_run:
                        connection.rollback()
                    else:
                        connection.commit()
                except BaseException:
                    connection.rollback()
                    raise
                if not dry_run:
                    completed.add(chunk_id)
                    checkpoint["completed"] = sorted(completed)
                    write_private(checkpoint_file, f"{canonical(checkpoint)}\n")
                applied += 1
                if interrupt_after is not None and applied >= interrupt_after:
                    raise InterruptedError("injected interruption")
    finally:
        connection.close()
    return applied, skipped


def reconcile_bundle(target: pathlib.Path, bundle: pathlib.Path) -> dict[str, Any]:
    manifest = load_manifest(bundle)
    connection = sqlite3.connect(target)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA foreign_keys = ON")
    tables: list[dict[str, Any]] = []
    total_mismatches = 0
    try:
        for table in manifest["tables"]:
            expected: dict[tuple[Any, ...], str] = {}
            for chunk in table["chunks"]:
                for row in read_chunk(bundle, chunk):
                    key = tuple(row[column] for column in table["primary_key"])
                    expected[key] = digest_row(row)
            columns = ", ".join(quote_identifier(column) for column in table["columns"])
            order = ", ".join(quote_identifier(column) for column in table["primary_key"])
            query = f"SELECT {columns} FROM {quote_identifier(table['name'])} ORDER BY {order}"
            actual: dict[tuple[Any, ...], str] = {}
            for row in connection.execute(query):
                normalized = {column: row[column] for column in table["columns"]}
                key = tuple(normalized[column] for column in table["primary_key"])
                actual[key] = digest_row(normalized)
            missing = set(expected) - set(actual)
            extra = set(actual) - set(expected)
            changed = {key for key in expected.keys() & actual if expected[key] != actual[key]}
            mismatch_count = len(missing) + len(extra) + len(changed)
            total_mismatches += mismatch_count
            tables.append(
                {
                    "name": table["name"],
                    "expected_rows": len(expected),
                    "actual_rows": len(actual),
                    "missing": len(missing),
                    "extra": len(extra),
                    "field_hash_mismatches": len(changed),
                }
            )
        foreign_key_violations = len(connection.execute("PRAGMA foreign_key_check").fetchall())
        total_mismatches += foreign_key_violations
    finally:
        connection.close()
    return {
        "schema_version": SCHEMA_VERSION,
        "run_id": manifest["run_id"],
        "tables": tables,
        "foreign_key_violations": foreign_key_violations,
        "unexplained_mismatches": total_mismatches,
        "clean": total_mismatches == 0,
    }


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    export = commands.add_parser("export", help="create an immutable credential-free bundle")
    export.add_argument("--source", type=pathlib.Path, required=True)
    export.add_argument("--plan", type=pathlib.Path, required=True)
    export.add_argument("--bundle", type=pathlib.Path, required=True)
    export.add_argument("--chunk-rows", type=int, default=1_000)
    import_command = commands.add_parser("import", help="idempotently import or resume a bundle")
    import_command.add_argument("--target", type=pathlib.Path, required=True)
    import_command.add_argument("--bundle", type=pathlib.Path, required=True)
    import_command.add_argument("--dry-run", action="store_true")
    import_command.add_argument("--inject-interruption-after", type=int, help=argparse.SUPPRESS)
    reconcile = commands.add_parser("reconcile", help="compare row and relationship evidence")
    reconcile.add_argument("--target", type=pathlib.Path, required=True)
    reconcile.add_argument("--bundle", type=pathlib.Path, required=True)
    reconcile.add_argument("--report", type=pathlib.Path)
    return root


def main(arguments: list[str] | None = None) -> int:
    args = parser().parse_args(arguments)
    try:
        if args.command == "export":
            manifest = export_bundle(args.source, load_plan(args.plan), args.bundle, args.chunk_rows)
            print(
                f"exported {manifest['row_count']} rows in {manifest['table_count']} tables; "
                f"quarantined {manifest['reject_count']} rows"
            )
            return 0 if manifest["reject_count"] == 0 else 3
        if args.command == "import":
            applied, skipped = import_bundle(
                args.target,
                args.bundle,
                args.dry_run,
                args.inject_interruption_after,
            )
            print(f"validated/applied {applied} chunks; resumed past {skipped} completed chunks")
            return 0
        report = reconcile_bundle(args.target, args.bundle)
        payload = f"{canonical(report)}\n"
        if args.report:
            write_private(args.report, payload)
        print(
            f"reconciled {len(report['tables'])} tables; "
            f"unexplained mismatches: {report['unexplained_mismatches']}"
        )
        return 0 if report["clean"] else 4
    except InterruptedError:
        print("migration interrupted after a durable checkpoint; rerun to resume", file=sys.stderr)
        return 75
    except (MigrationError, OSError, sqlite3.Error) as error:
        print(f"migration failed safely: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
