#!/usr/bin/env python3
"""Credential-free, deterministic metadata ETL rehearsal for MySQL-to-D1 cutovers.

This module intentionally speaks NDJSON and SQLite.  A protected MySQL exporter can
produce the same source envelope, while local and CI rehearsals can prove the bundle,
transform, checkpoint, idempotency, and reconciliation contracts without credentials.
No row values or primary keys are written to reports or operator messages.
"""

from __future__ import annotations

import argparse
import datetime as dt
import decimal
import hashlib
import json
import os
import pathlib
import re
import sqlite3
import sys
import tempfile
import time
import unicodedata
from dataclasses import dataclass
from typing import Any, Iterable, Iterator, Mapping, Sequence


SCHEMA_VERSION = 1
MAX_WIRE_INTEGER = 9_007_199_254_740_991
IDENTIFIER = re.compile(r"^[a-z][a-z0-9_]{0,62}$")
SAFE_LABEL = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:/@+-]{0,127}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
MISSING = object()


class EtlError(Exception):
    """An operator-safe failure which must never contain source values."""


class InjectedInterruption(Exception):
    """A deliberate interruption after a committed checkpoint."""


@dataclass(frozen=True)
class Column:
    source: str
    target: str
    transform: str
    nullable: bool
    options: Mapping[str, Any]
    has_default: bool
    default: Any


@dataclass(frozen=True)
class Table:
    name: str
    tenant_column: str
    primary_key: tuple[str, ...]
    columns: tuple[Column, ...]
    foreign_keys: tuple[Mapping[str, Any], ...]
    aggregates: tuple[Mapping[str, Any], ...]


@dataclass(frozen=True)
class Plan:
    raw: Mapping[str, Any]
    run_id: str
    source_schema: str
    target_migration: str
    code_sha: str
    window_start_ms: int
    window_end_ms: int
    tables: tuple[Table, ...]
    semantic_rules: tuple[Mapping[str, Any], ...]


def canonical(value: Any) -> str:
    return json.dumps(
        value,
        allow_nan=False,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_json(value: Any) -> str:
    return sha256_bytes(canonical(value).encode("utf-8"))


def tenant_digest(tenant: str) -> str:
    return sha256_bytes(f"frame-etl-tenant-v1\0{tenant}".encode("utf-8"))


def quote(identifier: str) -> str:
    if not isinstance(identifier, str) or not IDENTIFIER.fullmatch(identifier):
        raise EtlError("configuration contains an invalid SQL identifier")
    return f'"{identifier}"'


def private_directory(path: pathlib.Path, *, must_create: bool = False) -> None:
    if must_create:
        try:
            path.mkdir(mode=0o700, parents=True, exist_ok=False)
        except FileExistsError as error:
            raise EtlError("bundle path already exists; immutable exports are never overwritten") from error
    else:
        path.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.chmod(0o700)


def atomic_private_write(path: pathlib.Path, data: bytes) -> None:
    private_directory(path.parent)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary_path = pathlib.Path(temporary)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        temporary_path.replace(path)
        path.chmod(0o600)
    except BaseException:
        temporary_path.unlink(missing_ok=True)
        raise


def immutable_private_write(path: pathlib.Path, data: bytes) -> None:
    private_directory(path.parent)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    descriptor = os.open(path, flags, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
    except BaseException:
        path.unlink(missing_ok=True)
        raise


def require_label(raw: Any, field: str) -> str:
    if not isinstance(raw, str) or not SAFE_LABEL.fullmatch(raw):
        raise EtlError(f"plan {field} is invalid")
    return raw


def require_integer(raw: Any, field: str) -> int:
    if isinstance(raw, bool) or not isinstance(raw, int) or not 0 <= raw <= MAX_WIRE_INTEGER:
        raise EtlError(f"plan {field} is invalid")
    return raw


def load_json(path: pathlib.Path, description: str) -> Any:
    try:
        return strict_json(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
        raise EtlError(f"{description} is unreadable or invalid JSON") from error


def load_plan(path: pathlib.Path) -> Plan:
    raw = load_json(path, "ETL plan")
    if not isinstance(raw, dict) or raw.get("schema_version") != SCHEMA_VERSION:
        raise EtlError("plan schema version is unsupported")
    run_id = require_label(raw.get("run_id"), "run_id")
    source_schema = require_label(raw.get("source_schema"), "source_schema")
    target_migration = require_label(raw.get("target_migration"), "target_migration")
    code_sha = raw.get("code_sha")
    if not isinstance(code_sha, str) or not SHA256.fullmatch(code_sha):
        raise EtlError("plan code_sha must be an immutable SHA-256 digest")
    window = raw.get("window")
    if not isinstance(window, dict):
        raise EtlError("plan window is invalid")
    window_start_ms = require_integer(window.get("start_ms"), "window.start_ms")
    window_end_ms = require_integer(window.get("end_ms"), "window.end_ms")
    if window_start_ms > window_end_ms:
        raise EtlError("plan window is reversed")

    raw_tables = raw.get("tables")
    if not isinstance(raw_tables, list) or not raw_tables:
        raise EtlError("plan has no tables")
    tables: list[Table] = []
    seen_tables: set[str] = set()
    allowed_transforms = {
        "identity",
        "boolean",
        "canonical_json",
        "timestamp_ms",
        "wire_integer",
        "decimal_scaled",
        "casefold_nfkc",
        "enum",
    }
    for raw_table in raw_tables:
        if not isinstance(raw_table, dict):
            raise EtlError("plan table is invalid")
        name = raw_table.get("name")
        quote(name)
        if name in seen_tables:
            raise EtlError("plan repeats a table")
        seen_tables.add(name)
        tenant_column = raw_table.get("tenant_column")
        quote(tenant_column)
        raw_columns = raw_table.get("columns")
        if not isinstance(raw_columns, list) or not raw_columns:
            raise EtlError("plan table has no columns")
        columns: list[Column] = []
        seen_targets: set[str] = set()
        for raw_column in raw_columns:
            if not isinstance(raw_column, dict):
                raise EtlError("plan column is invalid")
            source = raw_column.get("source")
            target = raw_column.get("target")
            quote(source)
            quote(target)
            if target in seen_targets:
                raise EtlError("plan repeats a target column")
            seen_targets.add(target)
            transform = raw_column.get("transform", "identity")
            if transform not in allowed_transforms:
                raise EtlError("plan names an unsupported transform")
            options = raw_column.get("options", {})
            if not isinstance(options, dict):
                raise EtlError("plan transform options are invalid")
            if transform == "decimal_scaled":
                scale = options.get("scale")
                if isinstance(scale, bool) or not isinstance(scale, int) or not 0 <= scale <= 9:
                    raise EtlError("decimal scale is invalid")
            if transform == "enum":
                mapping = options.get("mapping")
                if not isinstance(mapping, dict) or not mapping:
                    raise EtlError("enum mapping is invalid")
                if not all(isinstance(key, str) and isinstance(value, str) for key, value in mapping.items()):
                    raise EtlError("enum mapping is invalid")
            has_default = "default" in raw_column
            columns.append(
                Column(
                    source=source,
                    target=target,
                    transform=transform,
                    nullable=raw_column.get("nullable") is True,
                    options=options,
                    has_default=has_default,
                    default=raw_column.get("default"),
                )
            )
        if tenant_column not in seen_targets:
            raise EtlError("tenant column must be present in transformed target columns")
        primary_key = raw_table.get("primary_key")
        if (
            not isinstance(primary_key, list)
            or not primary_key
            or not all(isinstance(item, str) and item in seen_targets for item in primary_key)
        ):
            raise EtlError("plan primary key is invalid")
        foreign_keys = raw_table.get("foreign_keys", [])
        aggregates = raw_table.get("aggregates", [])
        if not isinstance(foreign_keys, list) or not isinstance(aggregates, list):
            raise EtlError("plan reconciliation rules are invalid")
        for foreign_key in foreign_keys:
            _validate_foreign_key(foreign_key, seen_targets)
        for aggregate in aggregates:
            _validate_aggregate(aggregate, seen_targets)
        tables.append(
            Table(
                name=name,
                tenant_column=tenant_column,
                primary_key=tuple(primary_key),
                columns=tuple(columns),
                foreign_keys=tuple(foreign_keys),
                aggregates=tuple(aggregates),
            )
        )

    # Dependencies must refer backwards, making import order deterministic.
    prior: set[str] = set()
    for table in tables:
        for foreign_key in table.foreign_keys:
            referenced = foreign_key["references"]["table"]
            if referenced not in prior:
                raise EtlError("foreign-key dependency must appear earlier in the plan")
        prior.add(table.name)

    semantic_rules = raw.get("semantic_rules", [])
    if not isinstance(semantic_rules, list):
        raise EtlError("plan semantic rules are invalid")
    for rule in semantic_rules:
        _validate_semantic_rule(rule, seen_tables)
    table_columns = {
        table.name: {column.target for column in table.columns} for table in tables
    }
    for table in tables:
        for foreign_key in table.foreign_keys:
            referenced = foreign_key["references"]
            if not set(referenced["columns"]).issubset(table_columns[referenced["table"]]):
                raise EtlError("foreign-key rule references an unknown target column")
    for rule in semantic_rules:
        if rule["kind"] == "unique_per_tenant":
            if rule["column"] not in table_columns[rule["table"]]:
                raise EtlError("semantic rule references an unknown target column")
        elif rule["kind"] == "owner_membership":
            parent_columns = table_columns[rule["parent_table"]]
            membership_columns = table_columns[rule["membership_table"]]
            if not {rule["parent_id_column"], rule["owner_column"]}.issubset(parent_columns):
                raise EtlError("semantic rule references an unknown parent column")
            if not {
                rule["membership_parent_column"],
                rule["membership_user_column"],
                rule["role_column"],
                rule["state_column"],
            }.issubset(membership_columns):
                raise EtlError("semantic rule references an unknown membership column")
    return Plan(
        raw=raw,
        run_id=run_id,
        source_schema=source_schema,
        target_migration=target_migration,
        code_sha=code_sha,
        window_start_ms=window_start_ms,
        window_end_ms=window_end_ms,
        tables=tuple(tables),
        semantic_rules=tuple(semantic_rules),
    )


def _validate_foreign_key(raw: Any, local_columns: set[str]) -> None:
    if not isinstance(raw, dict) or set(raw) - {"columns", "references", "same_tenant"}:
        raise EtlError("foreign-key rule is invalid")
    columns = raw.get("columns")
    reference = raw.get("references")
    if (
        not isinstance(columns, list)
        or not columns
        or not all(isinstance(item, str) and item in local_columns for item in columns)
        or not isinstance(reference, dict)
        or set(reference) != {"table", "columns"}
        or not isinstance(reference.get("columns"), list)
        or len(reference["columns"]) != len(columns)
    ):
        raise EtlError("foreign-key rule is invalid")
    quote(reference.get("table"))
    for column in reference["columns"]:
        quote(column)
    if raw.get("same_tenant") not in (None, True, False):
        raise EtlError("foreign-key tenant scope is invalid")


def _validate_aggregate(raw: Any, local_columns: set[str]) -> None:
    if not isinstance(raw, dict) or set(raw) != {"name", "operation", "column"}:
        raise EtlError("aggregate rule is invalid")
    require_label(raw.get("name"), "aggregate.name")
    if raw.get("operation") not in {"count", "sum"}:
        raise EtlError("aggregate operation is invalid")
    if raw["operation"] == "sum" and raw.get("column") not in local_columns:
        raise EtlError("aggregate column is invalid")
    if raw["operation"] == "count" and raw.get("column") != "*":
        raise EtlError("count aggregate must use '*' column")


def _validate_semantic_rule(raw: Any, tables: set[str]) -> None:
    if not isinstance(raw, dict):
        raise EtlError("semantic rule is invalid")
    require_label(raw.get("name"), "semantic_rule.name")
    kind = raw.get("kind")
    if kind == "unique_per_tenant":
        if set(raw) != {"name", "kind", "table", "column"} or raw.get("table") not in tables:
            raise EtlError("unique semantic rule is invalid")
        quote(raw.get("column"))
        return
    if kind == "owner_membership":
        required = {
            "name",
            "kind",
            "parent_table",
            "parent_id_column",
            "owner_column",
            "membership_table",
            "membership_parent_column",
            "membership_user_column",
            "role_column",
            "required_role",
            "state_column",
            "required_state",
        }
        if set(raw) != required or raw.get("parent_table") not in tables or raw.get("membership_table") not in tables:
            raise EtlError("owner-membership semantic rule is invalid")
        for key in required - {"name", "kind", "parent_table", "membership_table", "required_role", "required_state"}:
            quote(raw[key])
        require_label(raw["required_role"], "semantic_rule.required_role")
        require_label(raw["required_state"], "semantic_rule.required_state")
        return
    raise EtlError("semantic rule kind is unsupported")


def _timestamp_ms(value: Any) -> int:
    if isinstance(value, bool):
        raise ValueError("invalid_timestamp")
    if isinstance(value, int):
        result = value
    elif isinstance(value, str):
        normalized = value[:-1] + "+00:00" if value.endswith("Z") else value
        parsed = dt.datetime.fromisoformat(normalized)
        if parsed.tzinfo is None:
            raise ValueError("timezone_required")
        utc = parsed.astimezone(dt.timezone.utc)
        epoch = dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc)
        delta = utc - epoch
        result = delta.days * 86_400_000 + delta.seconds * 1_000 + delta.microseconds // 1_000
    else:
        raise ValueError("invalid_timestamp")
    if not 0 <= result <= 253_402_300_799_999:
        raise ValueError("timestamp_out_of_range")
    return result


def transform_value(column: Column, value: Any) -> Any:
    if value is MISSING or value is None:
        if column.has_default:
            value = column.default
        elif column.nullable:
            return None
        else:
            raise ValueError("required_value_missing")
    kind = column.transform
    try:
        if kind == "identity":
            if not isinstance(value, (str, int)) or isinstance(value, bool):
                raise ValueError("invalid_identity_value")
            return value
        if kind == "boolean":
            if value in (True, 1, "1", "true", "TRUE"):
                return 1
            if value in (False, 0, "0", "false", "FALSE"):
                return 0
            raise ValueError("invalid_boolean")
        if kind == "canonical_json":
            decoded = strict_json(value) if isinstance(value, str) else value
            if not isinstance(decoded, (dict, list)):
                raise ValueError("invalid_json_shape")
            return canonical(decoded)
        if kind == "timestamp_ms":
            return _timestamp_ms(value)
        if kind == "wire_integer":
            integer = int(value)
            if str(integer) != str(value).strip() and not isinstance(value, int):
                raise ValueError("invalid_integer")
            if not -MAX_WIRE_INTEGER <= integer <= MAX_WIRE_INTEGER:
                raise ValueError("unsafe_wire_integer")
            return integer
        if kind == "decimal_scaled":
            scale = int(column.options["scale"])
            number = decimal.Decimal(str(value))
            quantum = decimal.Decimal(1).scaleb(-scale)
            quantized = number.quantize(quantum, rounding=decimal.ROUND_HALF_EVEN)
            if number != quantized:
                raise ValueError("decimal_precision_loss")
            scaled = int(quantized.scaleb(scale))
            if not -MAX_WIRE_INTEGER <= scaled <= MAX_WIRE_INTEGER:
                raise ValueError("decimal_out_of_range")
            return scaled
        if kind == "casefold_nfkc":
            if not isinstance(value, str):
                raise ValueError("invalid_collation_value")
            return unicodedata.normalize("NFKC", value).casefold().strip()
        if kind == "enum":
            mapping = column.options["mapping"]
            if not isinstance(value, str) or value not in mapping:
                raise ValueError("unknown_enum")
            return mapping[value]
    except (decimal.InvalidOperation, OverflowError, TypeError, ValueError) as error:
        if isinstance(error, ValueError) and str(error) in {
            "required_value_missing",
            "invalid_identity_value",
            "invalid_boolean",
            "invalid_json_shape",
            "invalid_timestamp",
            "timezone_required",
            "timestamp_out_of_range",
            "invalid_integer",
            "unsafe_wire_integer",
            "decimal_precision_loss",
            "decimal_out_of_range",
            "invalid_collation_value",
            "unknown_enum",
        }:
            raise
        raise ValueError(f"invalid_{kind}") from error
    raise ValueError("unsupported_transform")


def strict_json(value: str) -> Any:
    def object_pairs(pairs: Sequence[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, item in pairs:
            if key in result:
                raise ValueError("duplicate_json_key")
            result[key] = item
        return result

    def reject_constant(_value: str) -> None:
        raise ValueError("nonfinite_json_number")

    return json.loads(value, object_pairs_hook=object_pairs, parse_constant=reject_constant)


def transform_row(table: Table, tenant: str, source: Mapping[str, Any]) -> dict[str, Any]:
    transformed: dict[str, Any] = {}
    for column in table.columns:
        transformed[column.target] = transform_value(column, source.get(column.source, MISSING))
    tenant_value = transformed[table.tenant_column]
    if tenant_value != tenant:
        raise ValueError("tenant_scope_mismatch")
    if any(transformed[column] is None for column in table.primary_key):
        raise ValueError("missing_primary_key")
    return transformed


def read_source(payload: bytes, tables: Mapping[str, Table]) -> Iterator[tuple[int, str, str, Mapping[str, Any]]]:
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError as error:
        raise EtlError("source NDJSON is not UTF-8") from error
    for ordinal, line in enumerate(text.splitlines(), start=1):
        try:
            envelope = strict_json(line)
        except (json.JSONDecodeError, ValueError) as error:
            raise EtlError("source NDJSON contains invalid JSON") from error
        if not isinstance(envelope, dict) or set(envelope) != {"table", "tenant", "row"}:
            raise EtlError("source NDJSON envelope is invalid")
        table = envelope["table"]
        tenant = envelope["tenant"]
        row = envelope["row"]
        if table not in tables or not isinstance(tenant, str) or not SAFE_LABEL.fullmatch(tenant):
            raise EtlError("source NDJSON scope is invalid")
        if not isinstance(row, dict):
            raise EtlError("source NDJSON row is invalid")
        yield ordinal, table, tenant, row


def chunks(values: Sequence[dict[str, Any]], size: int) -> Iterator[Sequence[dict[str, Any]]]:
    for index in range(0, len(values), size):
        yield values[index : index + size]


def export_bundle(source: pathlib.Path, plan: Plan, bundle: pathlib.Path, chunk_rows: int) -> Mapping[str, Any]:
    if not 1 <= chunk_rows <= 100_000:
        raise EtlError("chunk_rows must be between 1 and 100000")
    private_directory(bundle, must_create=True)
    try:
        source_payload = source.read_bytes()
    except OSError as error:
        raise EtlError("source NDJSON is unreadable") from error
    tables = {table.name: table for table in plan.tables}
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = {}
    rejects: list[dict[str, Any]] = []
    seen_keys: set[tuple[str, str, tuple[Any, ...]]] = set()
    source_tenants: set[str] = set()
    for ordinal, table_name, tenant, raw_row in read_source(source_payload, tables):
        source_tenants.add(tenant)
        table = tables[table_name]
        try:
            transformed = transform_row(table, tenant, raw_row)
            logical_key = tuple(transformed[column] for column in table.primary_key)
            scoped_key = (table_name, tenant, logical_key)
            if scoped_key in seen_keys:
                raise ValueError("duplicate_logical_record")
            seen_keys.add(scoped_key)
            grouped.setdefault((table_name, tenant), []).append(transformed)
        except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
            reason = str(error).split(":", maxsplit=1)[0]
            if not SAFE_LABEL.fullmatch(reason):
                reason = "invalid_record"
            rejects.append(
                {
                    "ordinal": ordinal,
                    "reason_code": reason,
                    "table": table_name,
                    "tenant_digest": tenant_digest(tenant),
                }
            )

    manifest_tables: list[dict[str, Any]] = []
    for table in plan.tables:
        target_columns = [column.target for column in table.columns]
        tenant_sections: list[dict[str, Any]] = []
        # A zero-row scope is evidence too: it lets reconciliation detect target
        # extras for tables where a tenant had no source records.
        tenants = sorted(source_tenants)
        for tenant in tenants:
            rows = grouped.get((table.name, tenant), [])
            rows.sort(key=lambda row: tuple(canonical(row[column]) for column in table.primary_key))
            chunk_entries: list[dict[str, Any]] = []
            for chunk_index, chunk in enumerate(chunks(rows, chunk_rows), start=1):
                relative = pathlib.Path("chunks") / table.name / tenant_digest(tenant) / f"{chunk_index:06d}.ndjson"
                payload = "".join(f"{canonical(row)}\n" for row in chunk).encode("utf-8")
                immutable_private_write(bundle / relative, payload)
                chunk_entries.append(
                    {
                        "path": relative.as_posix(),
                        "rows": len(chunk),
                        "sha256": sha256_bytes(payload),
                    }
                )
            tenant_sections.append(
                {
                    "tenant": tenant,
                    "tenant_digest": tenant_digest(tenant),
                    "row_count": len(rows),
                    "chunks": chunk_entries,
                }
            )
        manifest_tables.append(
            {
                "name": table.name,
                "tenant_column": table.tenant_column,
                "columns": target_columns,
                "primary_key": list(table.primary_key),
                "foreign_keys": list(table.foreign_keys),
                "aggregates": list(table.aggregates),
                "row_count": sum(section["row_count"] for section in tenant_sections),
                "tenants": tenant_sections,
            }
        )

    if rejects:
        quarantine = "".join(f"{canonical(reject)}\n" for reject in rejects).encode("utf-8")
        immutable_private_write(bundle / "quarantine.ndjson", quarantine)
    manifest: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "run_id": plan.run_id,
        "source_schema": plan.source_schema,
        "target_migration": plan.target_migration,
        "code_sha": plan.code_sha,
        "window": {"start_ms": plan.window_start_ms, "end_ms": plan.window_end_ms},
        "source_sha256": sha256_bytes(source_payload),
        "plan_sha256": sha256_json(plan.raw),
        "chunk_rows": chunk_rows,
        "row_count": sum(table["row_count"] for table in manifest_tables),
        "reject_count": len(rejects),
        "tables": manifest_tables,
        "semantic_rules": list(plan.semantic_rules),
    }
    payload = f"{canonical(manifest)}\n".encode("utf-8")
    immutable_private_write(bundle / "manifest.json", payload)
    immutable_private_write(bundle / "manifest.sha256", f"{sha256_bytes(payload)}  manifest.json\n".encode("ascii"))
    return manifest


def load_manifest(bundle: pathlib.Path) -> Mapping[str, Any]:
    try:
        payload = (bundle / "manifest.json").read_bytes()
        checksum_line = (bundle / "manifest.sha256").read_text(encoding="ascii").strip()
        expected_checksum, expected_name = checksum_line.split("  ", maxsplit=1)
        manifest_text = payload.decode("utf-8")
        manifest = strict_json(manifest_text)
        if manifest_text != f"{canonical(manifest)}\n":
            raise EtlError("bundle manifest is not in canonical form")
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
        raise EtlError("bundle manifest is unreadable or invalid") from error
    if expected_name != "manifest.json" or not SHA256.fullmatch(expected_checksum):
        raise EtlError("bundle manifest checksum record is invalid")
    if sha256_bytes(payload) != expected_checksum:
        raise EtlError("bundle manifest checksum mismatch")
    if not isinstance(manifest, dict) or manifest.get("schema_version") != SCHEMA_VERSION:
        raise EtlError("bundle manifest version is unsupported")
    return manifest


def read_chunk(bundle: pathlib.Path, chunk: Mapping[str, Any]) -> list[dict[str, Any]]:
    try:
        relative = pathlib.PurePosixPath(chunk["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise EtlError("bundle contains an unsafe chunk path")
        payload = bundle.joinpath(*relative.parts).read_bytes()
        if sha256_bytes(payload) != chunk["sha256"]:
            raise EtlError("bundle chunk checksum mismatch")
        lines = payload.decode("utf-8").splitlines()
        rows = [strict_json(line) for line in lines]
        if any(f"{canonical(row)}" != line for row, line in zip(rows, lines, strict=True)):
            raise EtlError("bundle chunk is not in canonical form")
    except (KeyError, OSError, UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
        raise EtlError("bundle chunk is unreadable or invalid") from error
    if len(rows) != chunk.get("rows") or not all(isinstance(row, dict) for row in rows):
        raise EtlError("bundle chunk row count or shape differs from manifest")
    return rows


def _ensure_etl_ledger(connection: sqlite3.Connection) -> None:
    connection.executescript(
        """
        CREATE TABLE IF NOT EXISTS _frame_etl_checkpoints (
          run_id TEXT NOT NULL,
          tenant_digest TEXT NOT NULL CHECK(length(tenant_digest) = 64),
          table_name TEXT NOT NULL,
          chunk_sha256 TEXT NOT NULL CHECK(length(chunk_sha256) = 64),
          processed_rows INTEGER NOT NULL,
          committed_at_ms INTEGER NOT NULL,
          PRIMARY KEY(run_id, tenant_digest, table_name, chunk_sha256)
        );
        """
    )


def _insert_and_verify(connection: sqlite3.Connection, table: Mapping[str, Any], rows: Sequence[Mapping[str, Any]]) -> None:
    name = table["name"]
    columns = table["columns"]
    primary_key = table["primary_key"]
    for identifier in [name, *columns, *primary_key]:
        quote(identifier)
    column_sql = ", ".join(quote(column) for column in columns)
    placeholders = ", ".join("?" for _ in columns)
    conflict = ", ".join(quote(column) for column in primary_key)
    insert = f"INSERT INTO {quote(name)} ({column_sql}) VALUES ({placeholders}) ON CONFLICT ({conflict}) DO NOTHING"
    where = " AND ".join(f"{quote(column)} IS ?" for column in primary_key)
    select = f"SELECT {column_sql} FROM {quote(name)} WHERE {where}"
    for row in rows:
        if set(row) != set(columns):
            raise EtlError("bundle row shape differs from manifest")
        connection.execute(insert, tuple(row[column] for column in columns))
        key = tuple(row[column] for column in primary_key)
        existing = connection.execute(select, key).fetchone()
        if existing is None:
            raise EtlError("target row is missing after insert")
        actual = {column: existing[index] for index, column in enumerate(columns)}
        if sha256_json(actual) != sha256_json(row):
            raise EtlError("target key already contains different data")


def import_bundle(
    target: pathlib.Path,
    bundle: pathlib.Path,
    *,
    dry_run: bool,
    max_rows_per_second: int,
    interrupt_after_chunks: int | None = None,
) -> Mapping[str, int]:
    if max_rows_per_second < 0:
        raise EtlError("rate limit cannot be negative")
    manifest = load_manifest(bundle)
    connection = sqlite3.connect(target)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA foreign_keys = ON")
    if dry_run:
        connection.execute("BEGIN IMMEDIATE")
    else:
        _ensure_etl_ledger(connection)
        connection.commit()
    applied = 0
    skipped = 0
    applied_rows = 0
    started = time.monotonic()
    try:
        for table in manifest["tables"]:
            for tenant in table["tenants"]:
                for chunk in tenant["chunks"]:
                    checkpoint = None
                    if not dry_run:
                        checkpoint = connection.execute(
                            """SELECT 1 FROM _frame_etl_checkpoints
                               WHERE run_id = ? AND tenant_digest = ? AND table_name = ? AND chunk_sha256 = ?""",
                            (manifest["run_id"], tenant["tenant_digest"], table["name"], chunk["sha256"]),
                        ).fetchone()
                    if checkpoint:
                        skipped += 1
                        continue
                    rows = read_chunk(bundle, chunk)
                    if not dry_run:
                        connection.execute("BEGIN IMMEDIATE")
                    try:
                        _insert_and_verify(connection, table, rows)
                        if not dry_run:
                            connection.execute(
                                """INSERT INTO _frame_etl_checkpoints(
                                     run_id, tenant_digest, table_name, chunk_sha256, processed_rows, committed_at_ms
                                   ) VALUES (?, ?, ?, ?, ?, ?)""",
                                (
                                    manifest["run_id"],
                                    tenant["tenant_digest"],
                                    table["name"],
                                    chunk["sha256"],
                                    len(rows),
                                    manifest["window"]["end_ms"],
                                ),
                            )
                            connection.commit()
                    except BaseException:
                        connection.rollback()
                        raise
                    applied += 1
                    applied_rows += len(rows)
                    if not dry_run and interrupt_after_chunks is not None and applied >= interrupt_after_chunks:
                        raise InjectedInterruption
                    if max_rows_per_second and applied_rows:
                        desired_elapsed = applied_rows / max_rows_per_second
                        remaining = desired_elapsed - (time.monotonic() - started)
                        if remaining > 0:
                            time.sleep(remaining)
    finally:
        if dry_run and connection.in_transaction:
            connection.rollback()
        connection.close()
    return {"applied_chunks": applied, "skipped_chunks": skipped, "validated_rows": applied_rows}


def _expected_rows(bundle: pathlib.Path, manifest: Mapping[str, Any]) -> dict[str, dict[str, list[dict[str, Any]]]]:
    result: dict[str, dict[str, list[dict[str, Any]]]] = {}
    for table in manifest["tables"]:
        result[table["name"]] = {}
        for tenant in table["tenants"]:
            rows: list[dict[str, Any]] = []
            for chunk in tenant["chunks"]:
                rows.extend(read_chunk(bundle, chunk))
            result[table["name"]][tenant["tenant"]] = rows
    return result


def _actual_rows(
    connection: sqlite3.Connection, table: Mapping[str, Any], tenant: str
) -> list[dict[str, Any]]:
    columns = table["columns"]
    sql = (
        f"SELECT {', '.join(quote(column) for column in columns)} FROM {quote(table['name'])} "
        f"WHERE {quote(table['tenant_column'])} = ?"
    )
    return [dict(row) for row in connection.execute(sql, (tenant,))]


def _keyed(rows: Iterable[Mapping[str, Any]], primary_key: Sequence[str]) -> dict[tuple[Any, ...], str]:
    result: dict[tuple[Any, ...], str] = {}
    for row in rows:
        key = tuple(row[column] for column in primary_key)
        result[key] = sha256_json(row)
    return result


def _relationship_violations(
    rows: Mapping[str, Mapping[str, Sequence[Mapping[str, Any]]]],
    tables: Sequence[Mapping[str, Any]],
) -> int:
    violations = 0
    table_lookup = {table["name"]: table for table in tables}
    for table in tables:
        for foreign_key in table["foreign_keys"]:
            referenced_table = table_lookup[foreign_key["references"]["table"]]
            for tenant, local_rows in rows[table["name"]].items():
                if foreign_key.get("same_tenant", True):
                    referenced_rows = rows[referenced_table["name"]].get(tenant, [])
                else:
                    referenced_rows = [row for values in rows[referenced_table["name"]].values() for row in values]
                reference_keys = {
                    tuple(row[column] for column in foreign_key["references"]["columns"])
                    for row in referenced_rows
                }
                for row in local_rows:
                    key = tuple(row[column] for column in foreign_key["columns"])
                    if any(value is None for value in key):
                        continue
                    if key not in reference_keys:
                        violations += 1
    return violations


def _aggregate_mismatches(
    expected: Mapping[str, Mapping[str, Sequence[Mapping[str, Any]]]],
    actual: Mapping[str, Mapping[str, Sequence[Mapping[str, Any]]]],
    tables: Sequence[Mapping[str, Any]],
) -> list[dict[str, Any]]:
    report: list[dict[str, Any]] = []
    for table in tables:
        mismatch_count = 0
        for aggregate in table["aggregates"]:
            for tenant, expected_rows in expected[table["name"]].items():
                actual_rows = actual[table["name"]].get(tenant, [])
                if aggregate["operation"] == "count":
                    left, right = len(expected_rows), len(actual_rows)
                else:
                    column = aggregate["column"]
                    left = sum(row[column] for row in expected_rows if row[column] is not None)
                    right = sum(row[column] for row in actual_rows if row[column] is not None)
                mismatch_count += int(left != right)
        report.append({"table": table["name"], "aggregate_mismatches": mismatch_count})
    return report


def _semantic_violations(
    rows: Mapping[str, Mapping[str, Sequence[Mapping[str, Any]]]],
    rules: Sequence[Mapping[str, Any]],
) -> dict[str, int]:
    result: dict[str, int] = {}
    for rule in rules:
        violations = 0
        if rule["kind"] == "unique_per_tenant":
            for values in rows[rule["table"]].values():
                seen: set[Any] = set()
                for row in values:
                    value = row[rule["column"]]
                    if value in seen:
                        violations += 1
                    seen.add(value)
        elif rule["kind"] == "owner_membership":
            parent_rows = rows[rule["parent_table"]]
            membership_rows = rows[rule["membership_table"]]
            for tenant, parents in parent_rows.items():
                memberships = membership_rows.get(tenant, [])
                valid = {
                    (member[rule["membership_parent_column"]], member[rule["membership_user_column"]])
                    for member in memberships
                    if member[rule["role_column"]] == rule["required_role"]
                    and member[rule["state_column"]] == rule["required_state"]
                }
                for parent in parents:
                    key = (parent[rule["parent_id_column"]], parent[rule["owner_column"]])
                    if key not in valid:
                        violations += 1
        result[rule["name"]] = violations
    return result


def reconcile_bundle(target: pathlib.Path, bundle: pathlib.Path) -> Mapping[str, Any]:
    manifest = load_manifest(bundle)
    expected = _expected_rows(bundle, manifest)
    connection = sqlite3.connect(target)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA foreign_keys = ON")
    actual: dict[str, dict[str, list[dict[str, Any]]]] = {}
    sections: list[dict[str, Any]] = []
    mismatch_total = 0
    try:
        for table in manifest["tables"]:
            actual[table["name"]] = {}
            for tenant in table["tenants"]:
                tenant_value = tenant["tenant"]
                expected_rows = expected[table["name"]][tenant_value]
                actual_rows = _actual_rows(connection, table, tenant_value)
                actual[table["name"]][tenant_value] = actual_rows
                expected_keyed = _keyed(expected_rows, table["primary_key"])
                actual_keyed = _keyed(actual_rows, table["primary_key"])
                missing = len(expected_keyed.keys() - actual_keyed.keys())
                extra = len(actual_keyed.keys() - expected_keyed.keys())
                changed = sum(
                    expected_keyed[key] != actual_keyed[key]
                    for key in expected_keyed.keys() & actual_keyed.keys()
                )
                mismatch_total += missing + extra + changed
                sections.append(
                    {
                        "table": table["name"],
                        "tenant_digest": tenant["tenant_digest"],
                        "expected_rows": len(expected_rows),
                        "actual_rows": len(actual_rows),
                        "missing_primary_keys": missing,
                        "extra_primary_keys": extra,
                        "field_hash_mismatches": changed,
                    }
                )
        expected_relationship = _relationship_violations(expected, manifest["tables"])
        actual_relationship = _relationship_violations(actual, manifest["tables"])
        sqlite_foreign_keys = len(connection.execute("PRAGMA foreign_key_check").fetchall())
    finally:
        connection.close()
    aggregate_report = _aggregate_mismatches(expected, actual, manifest["tables"])
    aggregate_mismatches = sum(section["aggregate_mismatches"] for section in aggregate_report)
    expected_semantics = _semantic_violations(expected, manifest["semantic_rules"])
    actual_semantics = _semantic_violations(actual, manifest["semantic_rules"])
    semantic_report = [
        {
            "name": name,
            "expected_violations": expected_semantics[name],
            "actual_violations": actual_semantics[name],
            "mismatch": expected_semantics[name] != actual_semantics[name]
            or expected_semantics[name] != 0
            or actual_semantics[name] != 0,
        }
        for name in sorted(expected_semantics)
    ]
    semantic_mismatches = sum(section["mismatch"] for section in semantic_report)
    relationship_mismatches = expected_relationship + actual_relationship + sqlite_foreign_keys
    unexplained = mismatch_total + relationship_mismatches + aggregate_mismatches + semantic_mismatches
    return {
        "schema_version": SCHEMA_VERSION,
        "run_id": manifest["run_id"],
        "manifest_sha256": sha256_bytes((bundle / "manifest.json").read_bytes()),
        "tables": sections,
        "relationships": {
            "source_violations": expected_relationship,
            "target_violations": actual_relationship,
            "target_engine_violations": sqlite_foreign_keys,
        },
        "aggregates": aggregate_report,
        "semantics": semantic_report,
        "unexplained_mismatches": unexplained,
        "clean": unexplained == 0 and manifest["reject_count"] == 0,
        "blocked_by_quarantine": manifest["reject_count"] > 0,
    }


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    export = commands.add_parser("export", help="write a new immutable ETL bundle")
    export.add_argument("--source", type=pathlib.Path, required=True)
    export.add_argument("--plan", type=pathlib.Path, required=True)
    export.add_argument("--bundle", type=pathlib.Path, required=True)
    export.add_argument("--chunk-rows", type=int, default=1_000)
    import_command = commands.add_parser("import", help="validate or resumably import a bundle")
    import_command.add_argument("--target", type=pathlib.Path, required=True)
    import_command.add_argument("--bundle", type=pathlib.Path, required=True)
    import_command.add_argument("--dry-run", action="store_true")
    import_command.add_argument("--max-rows-per-second", type=int, default=0)
    import_command.add_argument("--inject-interruption-after-chunks", type=int, help=argparse.SUPPRESS)
    reconcile = commands.add_parser("reconcile", help="emit redacted row/relationship/semantic evidence")
    reconcile.add_argument("--target", type=pathlib.Path, required=True)
    reconcile.add_argument("--bundle", type=pathlib.Path, required=True)
    reconcile.add_argument("--report", type=pathlib.Path, required=True)
    return root


def main(arguments: Sequence[str] | None = None) -> int:
    args = parser().parse_args(arguments)
    try:
        if args.command == "export":
            manifest = export_bundle(args.source, load_plan(args.plan), args.bundle, args.chunk_rows)
            print(
                f"exported {manifest['row_count']} transformed rows; "
                f"quarantined {manifest['reject_count']} records"
            )
            return 0 if manifest["reject_count"] == 0 else 3
        if args.command == "import":
            result = import_bundle(
                args.target,
                args.bundle,
                dry_run=args.dry_run,
                max_rows_per_second=args.max_rows_per_second,
                interrupt_after_chunks=args.inject_interruption_after_chunks,
            )
            print(
                f"validated/applied {result['applied_chunks']} chunks; "
                f"resumed past {result['skipped_chunks']} durable checkpoints"
            )
            return 0
        report = reconcile_bundle(args.target, args.bundle)
        atomic_private_write(args.report, f"{canonical(report)}\n".encode("utf-8"))
        print(
            f"reconciled {len(report['tables'])} tenant/table scopes; "
            f"unexplained mismatches: {report['unexplained_mismatches']}"
        )
        return 0 if report["clean"] else 4
    except InjectedInterruption:
        print("ETL interrupted after a durable checkpoint; rerun the same bundle to resume", file=sys.stderr)
        return 75
    except (EtlError, OSError, sqlite3.Error) as error:
        print(f"ETL failed safely: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
