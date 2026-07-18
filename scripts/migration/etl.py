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
import importlib.util
import json
import os
import pathlib
import re
import sqlite3
import stat
import sys
import tempfile
import time
import unicodedata
from dataclasses import dataclass
from typing import Any, BinaryIO, Iterator, Mapping, Sequence

SCHEMA_VERSION = 1
MYSQL_SNAPSHOT_ATTESTATION_KIND = "mysql_snapshot_v1"
MYSQL_SNAPSHOT_BOUNDARY = "snapshot-boundary.protected.json"
MYSQL_SNAPSHOT_PROOF = "snapshot-proof.json"
MAX_SNAPSHOT_ATTESTATION_BYTES = 8 * 1024 * 1024
MAX_SOURCE_ROW_BYTES = 16 * 1024 * 1024
MAX_CHUNK_BYTES = 64 * 1024 * 1024
MAX_MANIFEST_BYTES = 64 * 1024 * 1024
MAX_MANIFEST_SECTIONS = 100_000
MAX_MANIFEST_CHUNKS = 100_000
MAX_PLAN_BYTES = 8 * 1024 * 1024
MAX_PLAN_TABLES = 128
MAX_PLAN_COLUMNS_PER_TABLE = 256
MAX_PLAN_RECONCILIATION_RULES = 512
MAX_PLAN_ENUM_VALUES = 1_024
MAX_WIRE_INTEGER = 9_007_199_254_740_991
IDENTIFIER = re.compile(r"^[a-z][a-z0-9_]{0,62}$")
SAFE_LABEL = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:/@+-]{0,127}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
MISSING = object()


def _load_cap_id_mapper():
    path = pathlib.Path(__file__).resolve().with_name("cap_id_map.py")
    specification = importlib.util.spec_from_file_location("frame_cap_id_map_v1", path)
    if specification is None or specification.loader is None:
        raise RuntimeError("Cap ID mapping contract could not be loaded")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    mapper = getattr(module, "map_cap_nanoid", None)
    if not callable(mapper):
        raise RuntimeError("Cap ID mapping contract is invalid")
    return mapper


map_cap_nanoid = _load_cap_id_mapper()


class EtlError(Exception):
    """An operator-safe failure which must never contain source values."""


class InjectedInterruption(Exception):
    """A deliberate interruption after a committed checkpoint."""


@dataclass(frozen=True)
class Column:
    source: str | None
    target: str
    transform: str
    nullable: bool
    options: Mapping[str, Any]
    has_default: bool
    default: Any
    sources: tuple[str, ...] = ()

    def input_sources(self) -> tuple[str, ...]:
        if self.sources:
            return self.sources
        return () if self.source is None else (self.source,)


@dataclass(frozen=True)
class Table:
    name: str
    tenant_column: str | None
    primary_key: tuple[str, ...]
    columns: tuple[Column, ...]
    foreign_keys: tuple[Mapping[str, Any], ...]
    aggregates: tuple[Mapping[str, Any], ...]
    target_table: str | None = None
    tenant_source: str | None = None
    tenant_transform: str = "identity"
    import_order: tuple[str, ...] = ()

    @property
    def target_name(self) -> str:
        return self.target_table or self.name


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


def bounded_regular_bytes(
    path: pathlib.Path,
    maximum_bytes: int,
    description: str,
    *,
    allow_empty: bool = False,
) -> bytes:
    descriptor: int | None = None
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_NOFOLLOW", 0),
        )
        metadata = os.fstat(descriptor)
        minimum = 0 if allow_empty else 1
        if not stat.S_ISREG(metadata.st_mode) or not minimum <= metadata.st_size <= maximum_bytes:
            raise EtlError(f"{description} is missing, unsafe, or outside its size bound")
        with os.fdopen(descriptor, "rb", closefd=True) as handle:
            descriptor = None
            payload = handle.read(maximum_bytes + 1)
            final_metadata = os.fstat(handle.fileno())
        if len(payload) != metadata.st_size or final_metadata.st_size != metadata.st_size:
            raise EtlError(f"{description} changed while it was being read")
        return payload
    except OSError as error:
        raise EtlError(f"{description} is unreadable") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)


def load_json(path: pathlib.Path, description: str) -> Any:
    try:
        return strict_json(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
        raise EtlError(f"{description} is unreadable or invalid JSON") from error


def load_plan(path: pathlib.Path) -> Plan:
    try:
        plan_payload = bounded_regular_bytes(path, MAX_PLAN_BYTES, "ETL plan")
        raw = strict_json(plan_payload.decode("utf-8"))
    except (UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
        raise EtlError("ETL plan is unreadable or invalid JSON") from error
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
    if (
        not isinstance(raw_tables, list)
        or not raw_tables
        or len(raw_tables) > MAX_PLAN_TABLES
    ):
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
        "cap_nanoid_uuid_v1",
        "enum",
        "sha256_digest",
        "canonical_json_sha256",
        "canonical_checksum",
        "composite",
        "date_utc",
        "microseconds",
        "constant",
        "versioned_json",
        "versioned_json_sha256",
        "canonical_object",
        "canonical_object_sha256",
        "milliseconds",
        "presence_enum",
    }
    for raw_table in raw_tables:
        if not isinstance(raw_table, dict):
            raise EtlError("plan table is invalid")
        name = raw_table.get("name")
        quote(name)
        if name in seen_tables:
            raise EtlError("plan repeats a table")
        seen_tables.add(name)
        target_table = raw_table.get("target_table", name)
        quote(target_table)
        tenant_column = raw_table.get("tenant_column")
        if tenant_column is not None:
            quote(tenant_column)
        raw_columns = raw_table.get("columns")
        if (
            not isinstance(raw_columns, list)
            or not raw_columns
            or len(raw_columns) > MAX_PLAN_COLUMNS_PER_TABLE
        ):
            raise EtlError("plan table has no columns")
        columns: list[Column] = []
        seen_targets: set[str] = set()
        for raw_column in raw_columns:
            if not isinstance(raw_column, dict):
                raise EtlError("plan column is invalid")
            target = raw_column.get("target")
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
            option_transforms = {
                "decimal_scaled",
                "enum",
                "sha256_digest",
                "canonical_checksum",
                "composite",
                "versioned_json",
                "versioned_json_sha256",
                "canonical_object",
                "canonical_object_sha256",
                "presence_enum",
            }
            if transform not in option_transforms and options:
                raise EtlError("plan transform options are invalid")
            if transform == "decimal_scaled":
                if set(options) != {"scale"}:
                    raise EtlError("decimal scale is invalid")
                scale = options.get("scale")
                if isinstance(scale, bool) or not isinstance(scale, int) or not 0 <= scale <= 9:
                    raise EtlError("decimal scale is invalid")
            if transform == "enum":
                if set(options) != {"mapping"}:
                    raise EtlError("enum mapping is invalid")
                mapping = options.get("mapping")
                if (
                    not isinstance(mapping, dict)
                    or not mapping
                    or len(mapping) > MAX_PLAN_ENUM_VALUES
                ):
                    raise EtlError("enum mapping is invalid")
                if not all(isinstance(key, str) and isinstance(value, str) for key, value in mapping.items()):
                    raise EtlError("enum mapping is invalid")
            has_default = "default" in raw_column
            raw_source = raw_column.get("source")
            if isinstance(raw_source, str):
                quote(raw_source)
                source = raw_source
                sources = (raw_source,)
            elif isinstance(raw_source, list):
                if (
                    not raw_source
                    or len(raw_source) > 32
                    or not all(isinstance(item, str) for item in raw_source)
                    or len(raw_source) != len(set(raw_source))
                ):
                    raise EtlError("plan column source list is invalid")
                for item in raw_source:
                    quote(item)
                source = raw_source[0]
                sources = tuple(raw_source)
            elif raw_source is None and transform == "constant" and has_default:
                source = None
                sources = ()
            else:
                raise EtlError("plan column source is invalid")
            if transform in {
                "composite",
                "canonical_checksum",
                "canonical_object",
                "canonical_object_sha256",
            } and len(sources) < 2:
                raise EtlError("plan multi-source transform requires multiple sources")
            if transform not in {
                "composite",
                "canonical_checksum",
                "canonical_object",
                "canonical_object_sha256",
            } and len(sources) > 1:
                raise EtlError("plan transform does not accept multiple sources")
            if transform == "constant" and (sources or not has_default):
                raise EtlError("plan constant transform is invalid")
            if transform == "sha256_digest":
                if set(options) - {"domain"}:
                    raise EtlError("plan digest options are invalid")
                if "domain" in options:
                    require_label(options["domain"], "digest.domain")
            if transform == "canonical_checksum":
                if set(options) - {"domain", "labels"}:
                    raise EtlError("plan checksum options are invalid")
                if "domain" in options:
                    require_label(options["domain"], "checksum.domain")
                labels = options.get("labels")
                if labels is not None and (
                    not isinstance(labels, list)
                    or len(labels) != len(sources)
                    or not all(isinstance(label, str) and SAFE_LABEL.fullmatch(label) for label in labels)
                ):
                    raise EtlError("plan checksum labels are invalid")
            if transform == "composite":
                if set(options) - {"delimiter", "prefix"}:
                    raise EtlError("plan composite options are invalid")
                delimiter = options.get("delimiter", ":")
                prefix = options.get("prefix", "")
                if (
                    not isinstance(delimiter, str)
                    or not 1 <= len(delimiter) <= 8
                    or not isinstance(prefix, str)
                    or len(prefix) > 64
                ):
                    raise EtlError("plan composite options are invalid")
            if transform in {"versioned_json", "versioned_json_sha256"}:
                if set(options) != {"schema_version"}:
                    raise EtlError("plan versioned JSON options are invalid")
                version = options["schema_version"]
                if isinstance(version, bool) or not isinstance(version, int) or not 1 <= version <= 65535:
                    raise EtlError("plan versioned JSON options are invalid")
            if transform in {"canonical_object", "canonical_object_sha256"}:
                if set(options) - {"labels", "schema_version"}:
                    raise EtlError("plan canonical object options are invalid")
                labels = options.get("labels")
                if (
                    not isinstance(labels, list)
                    or len(labels) != len(sources)
                    or len(labels) != len(set(labels))
                    or not all(isinstance(label, str) and SAFE_LABEL.fullmatch(label) for label in labels)
                ):
                    raise EtlError("plan canonical object labels are invalid")
                version = options.get("schema_version")
                if version is not None and (
                    isinstance(version, bool) or not isinstance(version, int) or not 1 <= version <= 65535
                ):
                    raise EtlError("plan canonical object version is invalid")
            if transform == "presence_enum":
                if set(options) != {"missing", "present"} or not all(
                    isinstance(options[key], str) and SAFE_LABEL.fullmatch(options[key])
                    for key in ("missing", "present")
                ):
                    raise EtlError("plan presence enum options are invalid")
            columns.append(
                Column(
                    source=source,
                    target=target,
                    transform=transform,
                    nullable=raw_column.get("nullable") is True,
                    options=options,
                    has_default=has_default,
                    default=raw_column.get("default"),
                    sources=sources,
                )
            )
        tenant_source = raw_table.get("tenant_source")
        tenant_transform = raw_table.get("tenant_transform", "identity")
        if tenant_source is not None:
            quote(tenant_source)
        if tenant_transform not in {"identity", "cap_nanoid_uuid_v1"}:
            raise EtlError("tenant transform is unsupported")
        if tenant_column not in seen_targets:
            if tenant_column is not None or tenant_source is None:
                raise EtlError("tenant scope must be a target column or explicit source")
        elif tenant_source is not None:
            tenant_columns = [column for column in columns if column.target == tenant_column]
            if len(tenant_columns) != 1 or tenant_columns[0].input_sources() != (tenant_source,):
                raise EtlError("tenant source differs from its target column mapping")
            if tenant_columns[0].transform != tenant_transform:
                raise EtlError("tenant transform differs from its target column mapping")
        primary_key = raw_table.get("primary_key")
        if (
            not isinstance(primary_key, list)
            or not primary_key
            or not all(isinstance(item, str) and item in seen_targets for item in primary_key)
        ):
            raise EtlError("plan primary key is invalid")
        import_order = raw_table.get("import_order", primary_key)
        if (
            not isinstance(import_order, list)
            or not import_order
            or len(import_order) > 32
            or not all(isinstance(item, str) and item in seen_targets for item in import_order)
            or len(import_order) != len(set(import_order))
        ):
            raise EtlError("plan import order is invalid")
        columns_by_target = {column.target: column for column in columns}
        if any(
            columns_by_target[item].nullable
            or columns_by_target[item].transform
            in {
                "canonical_json",
                "versioned_json",
                "canonical_object",
            }
            for item in import_order
        ):
            raise EtlError("plan import order must use required scalar columns")
        foreign_keys = raw_table.get("foreign_keys", [])
        aggregates = raw_table.get("aggregates", [])
        if (
            not isinstance(foreign_keys, list)
            or not isinstance(aggregates, list)
            or len(foreign_keys) + len(aggregates) > MAX_PLAN_RECONCILIATION_RULES
        ):
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
                target_table=target_table,
                tenant_source=tenant_source,
                tenant_transform=tenant_transform,
                import_order=tuple(import_order),
            )
        )

    # Dependencies must refer backwards, making import order deterministic.
    prior: set[str] = set()
    for table in tables:
        for foreign_key in table.foreign_keys:
            referenced = foreign_key["references"]["table"]
            if referenced not in prior:
                raise EtlError("foreign-key dependency must appear earlier in the plan")
        prior.add(table.target_name)

    semantic_rules = raw.get("semantic_rules", [])
    if (
        not isinstance(semantic_rules, list)
        or len(semantic_rules) > MAX_PLAN_RECONCILIATION_RULES
    ):
        raise EtlError("plan semantic rules are invalid")
    for rule in semantic_rules:
        _validate_semantic_rule(rule, seen_tables)
    table_columns: dict[str, set[str]] = {}
    target_shapes: dict[str, tuple[Any, ...]] = {}
    for table in tables:
        columns = {column.target for column in table.columns}
        shape = (
            table.tenant_column,
            table.primary_key,
            table.import_order,
            tuple(column.target for column in table.columns),
            table.foreign_keys,
            table.aggregates,
        )
        if table.target_name in target_shapes and target_shapes[table.target_name] != shape:
            raise EtlError("plan streams targeting one table must share an exact target shape")
        target_shapes[table.target_name] = shape
        table_columns[table.target_name] = columns
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
    kind = column.transform
    if value is MISSING or value is None:
        if column.has_default:
            value = column.default
        elif kind == "presence_enum" and value is None:
            return column.options["missing"]
        elif column.nullable:
            return None
        else:
            raise ValueError("required_value_missing")
    try:
        if kind == "identity":
            if not isinstance(value, (str, int)) or isinstance(value, bool):
                raise ValueError("invalid_identity_value")
            return value
        if kind == "boolean":
            if value is True or type(value) is int and value == 1 or value in ("1", "true", "TRUE"):
                return 1
            if value is False or type(value) is int and value == 0 or value in ("0", "false", "FALSE"):
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
            if isinstance(value, bool):
                raise ValueError("invalid_integer")
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
        if kind == "cap_nanoid_uuid_v1":
            if not isinstance(value, str):
                raise ValueError("invalid_cap_nanoid")
            return map_cap_nanoid(value)
        if kind == "enum":
            mapping = column.options["mapping"]
            if not isinstance(value, str) or value not in mapping:
                raise ValueError("unknown_enum")
            return mapping[value]
        if kind == "sha256_digest":
            if not isinstance(value, (str, int)) or isinstance(value, bool):
                raise ValueError("invalid_digest_value")
            domain = column.options.get("domain", "frame-etl-digest-v1")
            return sha256_bytes(f"{domain}\0{value}".encode("utf-8"))
        if kind == "canonical_json_sha256":
            decoded = strict_json(value) if isinstance(value, str) else value
            if not isinstance(decoded, (dict, list)):
                raise ValueError("invalid_json_shape")
            return sha256_json(decoded)
        if kind in {"versioned_json", "versioned_json_sha256"}:
            decoded = strict_json(value) if isinstance(value, str) else value
            if not isinstance(decoded, (dict, list)):
                raise ValueError("invalid_json_shape")
            document = {
                "schema_version": column.options["schema_version"],
                "payload": decoded,
            }
            return canonical(document) if kind == "versioned_json" else sha256_json(document)
        if kind == "canonical_checksum":
            if not isinstance(value, tuple) or any(item is MISSING for item in value):
                raise ValueError("required_value_missing")
            labels = column.options.get("labels")
            material: Any = (
                {label: item for label, item in zip(labels, value, strict=True)}
                if labels is not None
                else list(value)
            )
            domain = column.options.get("domain", "frame-etl-checksum-v1")
            return sha256_bytes(f"{domain}\0{canonical(material)}".encode("utf-8"))
        if kind == "composite":
            if (
                not isinstance(value, tuple)
                or any(item is MISSING or item is None for item in value)
                or any(not isinstance(item, (str, int)) or isinstance(item, bool) for item in value)
            ):
                raise ValueError("invalid_composite_value")
            delimiter = column.options.get("delimiter", ":")
            return f"{column.options.get('prefix', '')}{delimiter.join(str(item) for item in value)}"
        if kind in {"canonical_object", "canonical_object_sha256"}:
            if not isinstance(value, tuple) or any(item is MISSING for item in value):
                raise ValueError("required_value_missing")
            material = {
                label: item
                for label, item in zip(column.options["labels"], value, strict=True)
            }
            if "schema_version" in column.options:
                material = {
                    "schema_version": column.options["schema_version"],
                    "capabilities": material,
                }
            return canonical(material) if kind == "canonical_object" else sha256_json(material)
        if kind == "date_utc":
            if not isinstance(value, str):
                raise ValueError("invalid_date")
            try:
                parsed_date = dt.date.fromisoformat(value[:10])
            except ValueError as error:
                raise ValueError("invalid_date") from error
            if value not in {parsed_date.isoformat(), f"{parsed_date.isoformat()}T00:00:00Z"}:
                raise ValueError("invalid_date")
            return parsed_date.isoformat()
        if kind == "microseconds":
            if isinstance(value, bool):
                raise ValueError("invalid_microseconds")
            seconds = decimal.Decimal(str(value))
            micros = seconds * decimal.Decimal(1_000_000)
            if micros != micros.to_integral_value() or not 0 <= micros <= MAX_WIRE_INTEGER:
                raise ValueError("invalid_microseconds")
            return int(micros)
        if kind == "milliseconds":
            if isinstance(value, bool):
                raise ValueError("invalid_milliseconds")
            seconds = decimal.Decimal(str(value))
            millis = seconds * decimal.Decimal(1_000)
            if millis != millis.to_integral_value() or not 0 <= millis <= MAX_WIRE_INTEGER:
                raise ValueError("invalid_milliseconds")
            return int(millis)
        if kind == "presence_enum":
            return column.options["present"]
        if kind == "constant":
            return value
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
            "invalid_cap_nanoid",
            "unknown_enum",
            "invalid_digest_value",
            "invalid_composite_value",
            "invalid_date",
            "invalid_microseconds",
            "invalid_milliseconds",
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
        input_sources = column.input_sources()
        if not input_sources:
            raw_value: Any = MISSING
        elif len(input_sources) == 1:
            raw_value = source.get(input_sources[0], MISSING)
        else:
            raw_value = tuple(source.get(item, MISSING) for item in input_sources)
        transformed[column.target] = transform_value(column, raw_value)
    tenant_value = transform_tenant(table, tenant, source)
    if table.tenant_column is not None and transformed[table.tenant_column] != tenant_value:
        raise ValueError("tenant_scope_mismatch")
    if any(transformed[column] is None for column in table.primary_key):
        raise ValueError("missing_primary_key")
    return transformed


def transform_tenant(table: Table, source_tenant: str, source: Mapping[str, Any]) -> str:
    if table.tenant_column is not None:
        columns = [column for column in table.columns if column.target == table.tenant_column]
        if len(columns) != 1 or len(columns[0].input_sources()) != 1:
            raise ValueError("tenant_scope_invalid")
        column = columns[0]
        logical_source = column.input_sources()[0]
    else:
        if table.tenant_source is None:
            raise ValueError("tenant_scope_invalid")
        logical_source = table.tenant_source
        column = Column(
            source=logical_source,
            target="tenant_scope",
            transform=table.tenant_transform,
            nullable=False,
            options={},
            has_default=False,
            default=None,
        )
    raw_scope = source.get(logical_source, MISSING)
    if raw_scope != source_tenant:
        raise ValueError("tenant_scope_mismatch")
    transformed = transform_value(column, raw_scope)
    if not isinstance(transformed, str) or not SAFE_LABEL.fullmatch(transformed):
        raise ValueError("tenant_scope_invalid")
    return transformed


def read_source(
    source: pathlib.Path,
    tables: Mapping[str, Table],
    source_hasher: Any,
) -> Iterator[tuple[int, str, str, Mapping[str, Any]]]:
    try:
        descriptor = os.open(
            source,
            os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_NOFOLLOW", 0),
        )
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            os.close(descriptor)
            raise EtlError("source NDJSON must be a regular file")
        handle = os.fdopen(descriptor, "rb")
    except OSError as error:
        raise EtlError("source NDJSON is unreadable") from error
    with handle:
        ordinal = 0
        while raw_line := handle.readline(MAX_SOURCE_ROW_BYTES + 2):
            ordinal += 1
            source_hasher.update(raw_line)
            line_bytes = raw_line[:-1] if raw_line.endswith(b"\n") else raw_line
            if line_bytes.endswith(b"\r"):
                line_bytes = line_bytes[:-1]
            if len(line_bytes) > MAX_SOURCE_ROW_BYTES:
                raise EtlError("source NDJSON row exceeds the supported size")
            try:
                line = line_bytes.decode("utf-8")
            except UnicodeDecodeError as error:
                raise EtlError("source NDJSON is not UTF-8") from error
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


def _create_staging_database(path: pathlib.Path) -> sqlite3.Connection:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.close(descriptor)
    connection = sqlite3.connect(path)
    connection.execute("PRAGMA journal_mode = DELETE")
    connection.execute("PRAGMA synchronous = FULL")
    connection.executescript(
        """
        CREATE TABLE staged_tenants (
          tenant TEXT PRIMARY KEY NOT NULL
        ) WITHOUT ROWID;
        CREATE TABLE staged_rows (
          table_name TEXT NOT NULL,
          tenant TEXT NOT NULL,
          logical_key TEXT NOT NULL,
          import_order TEXT NOT NULL,
          payload TEXT NOT NULL,
          PRIMARY KEY(table_name, tenant, logical_key)
        ) WITHOUT ROWID;
        """
    )
    return connection


def _write_staged_chunks(
    connection: sqlite3.Connection,
    bundle: pathlib.Path,
    table_name: str,
    tenant: str,
    chunk_rows: int,
    import_order: Sequence[str],
) -> tuple[list[dict[str, Any]], int]:
    order_sql = ", ".join(
        f"json_extract(import_order, '$[{index}]')"
        for index in range(len(import_order))
    )
    cursor = connection.execute(
        f"""SELECT payload FROM staged_rows
           WHERE table_name = ? AND tenant = ? ORDER BY {order_sql}, logical_key""",
        (table_name, tenant),
    )
    entries: list[dict[str, Any]] = []
    chunk_index = 0
    total_rows = 0
    handle: BinaryIO | None = None
    hasher: Any = None
    current_rows = 0
    current_bytes = 0
    relative: pathlib.Path | None = None

    def finish_chunk() -> None:
        nonlocal handle, hasher, current_rows, current_bytes, relative
        if handle is None or relative is None:
            return
        handle.flush()
        os.fsync(handle.fileno())
        handle.close()
        entries.append(
            {
                "path": relative.as_posix(),
                "rows": current_rows,
                "sha256": hasher.hexdigest(),
            }
        )
        handle = None
        relative = None
        current_rows = 0
        current_bytes = 0

    try:
        for (payload,) in cursor:
            if handle is None:
                if chunk_index >= MAX_MANIFEST_CHUNKS:
                    raise EtlError("bundle exceeds the supported chunk count")
                chunk_index += 1
                relative = (
                    pathlib.Path("chunks")
                    / table_name
                    / tenant_digest(tenant)
                    / f"{chunk_index:06d}.ndjson"
                )
                destination = bundle / relative
                private_directory(destination.parent)
                descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                handle = os.fdopen(descriptor, "wb")
                hasher = hashlib.sha256()
            encoded = f"{payload}\n".encode("utf-8")
            if len(encoded) > MAX_CHUNK_BYTES:
                raise EtlError("transformed record exceeds the supported chunk size")
            if handle is not None and current_bytes + len(encoded) > MAX_CHUNK_BYTES:
                finish_chunk()
                if chunk_index >= MAX_MANIFEST_CHUNKS:
                    raise EtlError("bundle exceeds the supported chunk count")
                chunk_index += 1
                relative = (
                    pathlib.Path("chunks")
                    / table_name
                    / tenant_digest(tenant)
                    / f"{chunk_index:06d}.ndjson"
                )
                destination = bundle / relative
                private_directory(destination.parent)
                descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                handle = os.fdopen(descriptor, "wb")
                hasher = hashlib.sha256()
            handle.write(encoded)
            hasher.update(encoded)
            current_rows += 1
            current_bytes += len(encoded)
            total_rows += 1
            if current_rows == chunk_rows:
                finish_chunk()
        finish_chunk()
    except BaseException:
        if handle is not None:
            handle.close()
        raise
    return entries, total_rows


def export_bundle(
    source: pathlib.Path,
    plan: Plan,
    bundle: pathlib.Path,
    chunk_rows: int,
    *,
    source_attestation: Mapping[str, Any] | None = None,
) -> Mapping[str, Any]:
    if not 1 <= chunk_rows <= 100_000:
        raise EtlError("chunk_rows must be between 1 and 100000")
    if source_attestation is not None:
        _validate_snapshot_attestation_shape(source_attestation)
    tables = {table.name: table for table in plan.tables}
    source_hasher = hashlib.sha256()
    reject_count = 0
    with tempfile.TemporaryDirectory(prefix=".frame-etl-transform-", dir=bundle.parent) as raw:
        staging = pathlib.Path(raw)
        staging.chmod(0o700)
        connection = _create_staging_database(staging / "rows.sqlite")
        try:
            quarantine_path = staging / "quarantine.ndjson"
            quarantine_descriptor = os.open(
                quarantine_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600
            )
            quarantine_handle = os.fdopen(quarantine_descriptor, "wb")
            try:
                processed = 0
                for ordinal, table_name, tenant, raw_row in read_source(
                    source, tables, source_hasher
                ):
                    table = tables[table_name]
                    try:
                        transformed_tenant = transform_tenant(table, tenant, raw_row)
                        transformed = transform_row(table, tenant, raw_row)
                        connection.execute(
                            "INSERT OR IGNORE INTO staged_tenants(tenant) VALUES (?)",
                            (transformed_tenant,),
                        )
                        logical_key = canonical(
                            [transformed[column] for column in table.primary_key]
                        )
                        import_order = canonical(
                            [transformed[column] for column in table.import_order]
                        )
                        try:
                            connection.execute(
                                """INSERT INTO staged_rows(
                                     table_name, tenant, logical_key, import_order, payload
                                   ) VALUES (?, ?, ?, ?, ?)""",
                                (
                                    table.target_name,
                                    transformed_tenant,
                                    logical_key,
                                    import_order,
                                    canonical(transformed),
                                ),
                            )
                        except sqlite3.IntegrityError as error:
                            raise ValueError("duplicate_logical_record") from error
                    except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
                        reason = str(error).split(":", maxsplit=1)[0]
                        if not SAFE_LABEL.fullmatch(reason):
                            reason = "invalid_record"
                        reject = {
                            "ordinal": ordinal,
                            "reason_code": reason,
                            "table": table_name,
                            "tenant_digest": tenant_digest(tenant),
                        }
                        quarantine_handle.write(f"{canonical(reject)}\n".encode("utf-8"))
                        reject_count += 1
                    processed += 1
                    if processed % 10_000 == 0:
                        connection.commit()
                connection.commit()
                quarantine_handle.flush()
                os.fsync(quarantine_handle.fileno())
            finally:
                quarantine_handle.close()

            private_directory(bundle, must_create=True)
            manifest_tables: list[dict[str, Any]] = []
            section_count = 0
            chunk_count = 0
            target_streams: list[Table] = []
            seen_targets: set[str] = set()
            for table in plan.tables:
                if table.target_name not in seen_targets:
                    target_streams.append(table)
                    seen_targets.add(table.target_name)
            for table in target_streams:
                target_name = table.target_name
                target_columns = [column.target for column in table.columns]
                tenant_sections: list[dict[str, Any]] = []
                tenant_cursor = connection.execute(
                    "SELECT tenant FROM staged_tenants ORDER BY tenant"
                )
                for (tenant,) in tenant_cursor:
                    section_count += 1
                    if section_count > MAX_MANIFEST_SECTIONS:
                        raise EtlError("bundle exceeds the supported tenant-section count")
                    chunk_entries, row_count = _write_staged_chunks(
                        connection,
                        bundle,
                        target_name,
                        tenant,
                        chunk_rows,
                        table.import_order,
                    )
                    chunk_count += len(chunk_entries)
                    if chunk_count > MAX_MANIFEST_CHUNKS:
                        raise EtlError("bundle exceeds the supported chunk count")
                    tenant_sections.append(
                        {
                            "tenant": tenant,
                            "tenant_digest": tenant_digest(tenant),
                            "row_count": row_count,
                            "chunks": chunk_entries,
                        }
                    )
                manifest_tables.append(
                    {
                        "name": target_name,
                        "tenant_column": table.tenant_column,
                        "columns": target_columns,
                        "primary_key": list(table.primary_key),
                        "import_order": list(table.import_order),
                        "foreign_keys": list(table.foreign_keys),
                        "aggregates": list(table.aggregates),
                        "row_count": sum(
                            section["row_count"] for section in tenant_sections
                        ),
                        "tenants": tenant_sections,
                    }
                )
            if reject_count:
                os.rename(quarantine_path, bundle / "quarantine.ndjson")
            manifest: dict[str, Any] = {
                "schema_version": SCHEMA_VERSION,
                "run_id": plan.run_id,
                "source_schema": plan.source_schema,
                "target_migration": plan.target_migration,
                "code_sha": plan.code_sha,
                "window": {"start_ms": plan.window_start_ms, "end_ms": plan.window_end_ms},
                "source_sha256": source_hasher.hexdigest(),
                "plan_sha256": sha256_json(plan.raw),
                "chunk_rows": chunk_rows,
                "row_count": sum(table["row_count"] for table in manifest_tables),
                "reject_count": reject_count,
                "tables": manifest_tables,
                "semantic_rules": list(plan.semantic_rules),
            }
            if source_attestation is not None:
                manifest["source_attestation"] = dict(source_attestation)
            payload = f"{canonical(manifest)}\n".encode("utf-8")
            if len(payload) > MAX_MANIFEST_BYTES:
                raise EtlError("bundle manifest exceeds the supported size")
            immutable_private_write(bundle / "manifest.json", payload)
            immutable_private_write(
                bundle / "manifest.sha256",
                f"{sha256_bytes(payload)}  manifest.json\n".encode("ascii"),
            )
            return manifest
        finally:
            connection.close()


def _validate_snapshot_attestation_shape(attestation: Mapping[str, Any]) -> None:
    expected = {
        "kind",
        "boundary_path",
        "boundary_sha256",
        "proof_path",
        "proof_sha256",
        "query_sha256",
        "gtid_sha256",
        "source_binding_sha256",
        "manifest_core_sha256",
    }
    if set(attestation) != expected:
        raise EtlError("snapshot source attestation has an invalid shape")
    if (
        attestation.get("kind") != MYSQL_SNAPSHOT_ATTESTATION_KIND
        or attestation.get("boundary_path") != MYSQL_SNAPSHOT_BOUNDARY
        or attestation.get("proof_path") != MYSQL_SNAPSHOT_PROOF
    ):
        raise EtlError("snapshot source attestation has an unsupported identity")
    for field in (
        "boundary_sha256",
        "proof_sha256",
        "query_sha256",
        "gtid_sha256",
        "source_binding_sha256",
        "manifest_core_sha256",
    ):
        value = attestation.get(field)
        if not isinstance(value, str) or not SHA256.fullmatch(value):
            raise EtlError("snapshot source attestation contains an invalid digest")


def manifest_core_sha256(manifest: Mapping[str, Any]) -> str:
    core = dict(manifest)
    core.pop("source_attestation", None)
    return sha256_json(core)


def attach_source_attestation(
    bundle: pathlib.Path,
    manifest: Mapping[str, Any],
    source_attestation: Mapping[str, Any],
) -> Mapping[str, Any]:
    if "source_attestation" in manifest:
        raise EtlError("bundle manifest already has a source attestation")
    _validate_snapshot_attestation_shape(source_attestation)
    if source_attestation["manifest_core_sha256"] != manifest_core_sha256(manifest):
        raise EtlError("source attestation does not bind the manifest core")
    updated = dict(manifest)
    updated["source_attestation"] = dict(source_attestation)
    payload = f"{canonical(updated)}\n".encode("utf-8")
    if len(payload) > MAX_MANIFEST_BYTES:
        raise EtlError("bundle manifest exceeds the supported size")
    atomic_private_write(bundle / "manifest.json", payload)
    atomic_private_write(
        bundle / "manifest.sha256",
        f"{sha256_bytes(payload)}  manifest.json\n".encode("ascii"),
    )
    return updated


def _canonical_attestation_file(bundle: pathlib.Path, name: str) -> tuple[bytes, Mapping[str, Any]]:
    path = bundle / name
    try:
        payload = bounded_regular_bytes(
            path, MAX_SNAPSHOT_ATTESTATION_BYTES, "snapshot attestation file"
        )
        decoded = payload.decode("utf-8")
        value = strict_json(decoded)
    except (OSError, UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
        raise EtlError("snapshot attestation file is unreadable or invalid") from error
    if not isinstance(value, dict) or payload != f"{canonical(value)}\n".encode("utf-8"):
        raise EtlError("snapshot attestation file is not canonical")
    return payload, value


def _attestation_integer(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= MAX_WIRE_INTEGER:
        raise EtlError(f"snapshot attestation {label} is invalid")
    return value


def _validate_snapshot_attestation_files(
    bundle: pathlib.Path,
    manifest: Mapping[str, Any],
) -> None:
    attestation = manifest.get("source_attestation")
    if attestation is None:
        return
    if not isinstance(attestation, dict):
        raise EtlError("snapshot source attestation is invalid")
    _validate_snapshot_attestation_shape(attestation)
    boundary_payload, boundary = _canonical_attestation_file(bundle, MYSQL_SNAPSHOT_BOUNDARY)
    proof_payload, proof = _canonical_attestation_file(bundle, MYSQL_SNAPSHOT_PROOF)
    if (
        sha256_bytes(boundary_payload) != attestation["boundary_sha256"]
        or sha256_bytes(proof_payload) != attestation["proof_sha256"]
    ):
        raise EtlError("snapshot attestation file digest mismatch")

    boundary_fields = {
        "schema_version",
        "run_id",
        "snapshot_started_at_ms",
        "snapshot_completed_at_ms",
        "gtid_before_snapshot",
        "gtid_sha256",
        "query_sha256",
        "source_binding_sha256",
        "manifest_core_sha256",
        "source_sha256",
        "plan_sha256",
        "preflight_policy",
    }
    proof_fields = {
        "schema_version",
        "run_id",
        "snapshot_started_at_ms",
        "snapshot_completed_at_ms",
        "captured_row_count",
        "source_bytes",
        "gtid_sha256",
        "query_sha256",
        "source_binding_sha256",
        "manifest_core_sha256",
        "source_sha256",
        "plan_sha256",
        "preflight_policy",
        "contains_source_values",
        "production_evidence",
    }
    if set(boundary) != boundary_fields or set(proof) != proof_fields:
        raise EtlError("snapshot attestation payload has an invalid shape")
    if boundary.get("schema_version") != 1 or proof.get("schema_version") != 1:
        raise EtlError("snapshot attestation payload version is unsupported")
    started = _attestation_integer(boundary.get("snapshot_started_at_ms"), "start time")
    completed = _attestation_integer(boundary.get("snapshot_completed_at_ms"), "end time")
    _attestation_integer(proof.get("snapshot_started_at_ms"), "proof start time")
    _attestation_integer(proof.get("snapshot_completed_at_ms"), "proof end time")
    captured_row_count = _attestation_integer(
        proof.get("captured_row_count"), "captured row count"
    )
    manifest_row_count = _attestation_integer(manifest.get("row_count"), "manifest row count")
    manifest_reject_count = _attestation_integer(
        manifest.get("reject_count"), "manifest reject count"
    )
    _attestation_integer(proof.get("source_bytes"), "source size")
    window = manifest.get("window")
    if not isinstance(window, dict):
        raise EtlError("snapshot attestation cannot bind an invalid manifest window")
    window_start = _attestation_integer(window.get("start_ms"), "manifest window start")
    window_end = _attestation_integer(window.get("end_ms"), "manifest window end")
    if (
        started > completed
        or started < window_start
        or completed > window_end
        or proof["snapshot_started_at_ms"] != started
        or proof["snapshot_completed_at_ms"] != completed
        or captured_row_count != manifest_row_count + manifest_reject_count
    ):
        raise EtlError("snapshot attestation timing or row count differs from manifest")
    gtid = boundary.get("gtid_before_snapshot")
    if not isinstance(gtid, str) or not gtid or len(gtid.encode("utf-8")) > 4 * 1024 * 1024:
        raise EtlError("snapshot attestation GTID boundary is invalid")
    try:
        gtid_bytes = gtid.encode("ascii", errors="strict")
    except UnicodeEncodeError as error:
        raise EtlError("snapshot attestation GTID boundary is invalid") from error
    if sha256_bytes(gtid_bytes) != boundary.get("gtid_sha256"):
        raise EtlError("snapshot attestation GTID digest mismatch")
    common = (
        "run_id",
        "gtid_sha256",
        "query_sha256",
        "source_binding_sha256",
        "manifest_core_sha256",
        "source_sha256",
        "plan_sha256",
        "preflight_policy",
    )
    if any(boundary.get(field) != proof.get(field) for field in common):
        raise EtlError("snapshot proof and protected boundary disagree")
    for field in (
        "gtid_sha256",
        "query_sha256",
        "source_binding_sha256",
        "manifest_core_sha256",
        "source_sha256",
        "plan_sha256",
    ):
        if not isinstance(boundary.get(field), str) or not SHA256.fullmatch(boundary[field]):
            raise EtlError("snapshot attestation payload contains an invalid digest")
    if (
        boundary.get("run_id") != manifest.get("run_id")
        or boundary.get("manifest_core_sha256") != manifest_core_sha256(manifest)
        or boundary.get("manifest_core_sha256") != attestation.get("manifest_core_sha256")
        or boundary.get("source_sha256") != manifest.get("source_sha256")
        or boundary.get("plan_sha256") != manifest.get("plan_sha256")
        or boundary.get("preflight_policy") != "mysql_gtid_row_full_innodb_v2"
        or proof.get("contains_source_values") is not False
        or proof.get("production_evidence") is not False
    ):
        raise EtlError("snapshot attestation does not bind the ETL manifest")
    for field in (
        "query_sha256",
        "gtid_sha256",
        "source_binding_sha256",
        "manifest_core_sha256",
    ):
        if boundary.get(field) != attestation.get(field):
            raise EtlError("snapshot attestation digest differs from manifest")


def load_manifest(bundle: pathlib.Path) -> Mapping[str, Any]:
    try:
        manifest_path = bundle / "manifest.json"
        payload = bounded_regular_bytes(manifest_path, MAX_MANIFEST_BYTES, "bundle manifest")
        checksum_line = bounded_regular_bytes(
            bundle / "manifest.sha256", 256, "bundle manifest checksum"
        ).decode("ascii").strip()
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
    _validate_snapshot_attestation_files(bundle, manifest)
    return manifest


def read_chunk(bundle: pathlib.Path, chunk: Mapping[str, Any]) -> list[dict[str, Any]]:
    try:
        relative = pathlib.PurePosixPath(chunk["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise EtlError("bundle contains an unsafe chunk path")
        path = bundle.joinpath(*relative.parts)
        payload = bounded_regular_bytes(path, MAX_CHUNK_BYTES, "bundle chunk")
        if sha256_bytes(payload) != chunk["sha256"]:
            raise EtlError("bundle chunk checksum mismatch")
        if not payload.endswith(b"\n"):
            raise EtlError("bundle chunk lacks its terminal record delimiter")
        encoded_lines = payload[:-1].split(b"\n")
        lines = [line.decode("utf-8") for line in encoded_lines]
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


def _import_order_key(table: Mapping[str, Any], row: Mapping[str, Any]) -> tuple[str | int, ...]:
    order = table.get("import_order", table["primary_key"])
    if (
        not isinstance(order, list)
        or not order
        or any(column not in row for column in order)
    ):
        raise EtlError("bundle import order is invalid")
    values = tuple(row[column] for column in order)
    if any(isinstance(value, bool) or not isinstance(value, (str, int)) for value in values):
        raise EtlError("bundle import order contains a non-scalar value")
    return values


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
                previous_order: tuple[str | int, ...] | None = None
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
                    for row in rows:
                        current_order = _import_order_key(table, row)
                        try:
                            if previous_order is not None and current_order < previous_order:
                                raise EtlError("bundle rows violate their declared import order")
                        except TypeError as error:
                            raise EtlError("bundle import order has inconsistent scalar types") from error
                        previous_order = current_order
                    tenant_column = table.get("tenant_column")
                    if tenant_column is not None and any(
                        row.get(tenant_column) != tenant["tenant"] for row in rows
                    ):
                        raise EtlError("bundle row differs from its tenant section")
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


def _reconciliation_database(path: pathlib.Path) -> sqlite3.Connection:
    connection = _create_staging_database(path)
    connection.execute("PRAGMA secure_delete = ON")
    connection.executescript(
        """
        DROP TABLE staged_tenants;
        DROP TABLE staged_rows;
        CREATE TABLE reconciliation_scopes (
          side INTEGER NOT NULL CHECK(side IN (0, 1)),
          table_name TEXT NOT NULL,
          tenant TEXT NOT NULL,
          PRIMARY KEY(side, table_name, tenant)
        ) WITHOUT ROWID;
        CREATE TABLE reconciliation_rows (
          side INTEGER NOT NULL CHECK(side IN (0, 1)),
          table_name TEXT NOT NULL,
          tenant TEXT NOT NULL,
          logical_key TEXT NOT NULL,
          row_hash TEXT NOT NULL,
          payload TEXT NOT NULL,
          PRIMARY KEY(side, table_name, tenant, logical_key)
        ) WITHOUT ROWID;
        CREATE INDEX reconciliation_rows_scope
          ON reconciliation_rows(side, table_name, tenant);
        """
    )
    return connection


def _stage_reconciliation(
    staging: sqlite3.Connection,
    target: sqlite3.Connection,
    bundle: pathlib.Path,
    manifest: Mapping[str, Any],
) -> None:
    insert_scope = "INSERT OR IGNORE INTO reconciliation_scopes VALUES (?, ?, ?)"
    insert_row = "INSERT INTO reconciliation_rows VALUES (?, ?, ?, ?, ?, ?)"
    processed = 0
    for table in manifest["tables"]:
        name = table["name"]
        columns = table["columns"]
        primary_key = table["primary_key"]
        for tenant in table["tenants"]:
            tenant_value = tenant["tenant"]
            staging.execute(insert_scope, (0, name, tenant_value))
            for chunk in tenant["chunks"]:
                for row in read_chunk(bundle, chunk):
                    tenant_column = table.get("tenant_column")
                    if set(row) != set(columns) or (
                        tenant_column is not None and row[tenant_column] != tenant_value
                    ):
                        raise EtlError("bundle reconciliation row differs from its manifest scope")
                    logical_key = canonical([row[column] for column in primary_key])
                    try:
                        staging.execute(
                            insert_row,
                            (0, name, tenant_value, logical_key, sha256_json(row), canonical(row)),
                        )
                    except sqlite3.IntegrityError as error:
                        raise EtlError("bundle repeats a reconciliation logical record") from error
                    processed += 1
                    if processed % 10_000 == 0:
                        staging.commit()
        select = f"SELECT {', '.join(quote(column) for column in columns)} FROM {quote(name)}"
        for raw_row in target.execute(select):
            row = dict(raw_row)
            logical_key = canonical([row[column] for column in primary_key])
            expected_tenants = staging.execute(
                "SELECT tenant FROM reconciliation_rows "
                "WHERE side=0 AND table_name=? AND logical_key=? ORDER BY tenant LIMIT 2",
                (name, logical_key),
            ).fetchall()
            if len(expected_tenants) > 1:
                raise EtlError("bundle maps one target key to multiple tenants")
            if expected_tenants:
                tenant_value = expected_tenants[0][0]
            elif table.get("tenant_column") is not None:
                tenant_value = row[table["tenant_column"]]
            else:
                tenant_value = "target-only"
            if not isinstance(tenant_value, str) or not SAFE_LABEL.fullmatch(tenant_value):
                raise EtlError("target contains an invalid tenant scope")
            staging.execute(insert_scope, (1, name, tenant_value))
            staging.execute(
                insert_row,
                (1, name, tenant_value, logical_key, sha256_json(row), canonical(row)),
            )
            processed += 1
            if processed % 10_000 == 0:
                staging.commit()
    staging.commit()


def _json_value(alias: str, column: str) -> str:
    quote(column)
    return f"json_extract({alias}.payload, '$.{column}')"


def _relationship_violations_disk(
    staging: sqlite3.Connection,
    tables: Sequence[Mapping[str, Any]],
    side: int,
) -> int:
    violations = 0
    for table in tables:
        for foreign_key in table["foreign_keys"]:
            local_values = [
                _json_value("child", column) for column in foreign_key["columns"]
            ]
            remote_values = [
                _json_value("parent", column)
                for column in foreign_key["references"]["columns"]
            ]
            non_null = " AND ".join(f"{value} IS NOT NULL" for value in local_values)
            equality = " AND ".join(
                f"{left} IS {right}"
                for left, right in zip(local_values, remote_values, strict=True)
            )
            tenant = " AND parent.tenant = child.tenant" if foreign_key.get("same_tenant", True) else ""
            query = f"""
                SELECT COUNT(*) FROM reconciliation_rows AS child
                WHERE child.side = ? AND child.table_name = ? AND {non_null}
                  AND NOT EXISTS (
                    SELECT 1 FROM reconciliation_rows AS parent
                    WHERE parent.side = ? AND parent.table_name = ?{tenant} AND {equality}
                  )
            """
            violations += staging.execute(
                query,
                (
                    side,
                    table["name"],
                    side,
                    foreign_key["references"]["table"],
                ),
            ).fetchone()[0]
    return violations


def _aggregate_value(
    staging: sqlite3.Connection,
    table: str,
    tenant: str,
    side: int,
    aggregate: Mapping[str, Any],
) -> int:
    if aggregate["operation"] == "count":
        expression = "COUNT(*)"
    else:
        expression = f"COALESCE(SUM({_json_value('item', aggregate['column'])}), 0)"
    return staging.execute(
        f"SELECT {expression} FROM reconciliation_rows AS item "
        "WHERE item.side = ? AND item.table_name = ? AND item.tenant = ?",
        (side, table, tenant),
    ).fetchone()[0]


def _aggregate_mismatches_disk(
    staging: sqlite3.Connection,
    tables: Sequence[Mapping[str, Any]],
) -> list[dict[str, Any]]:
    report: list[dict[str, Any]] = []
    for table in tables:
        mismatch_count = 0
        tenants = staging.execute(
            "SELECT DISTINCT tenant FROM reconciliation_scopes WHERE table_name = ? ORDER BY tenant",
            (table["name"],),
        )
        for (tenant,) in tenants:
            for aggregate in table["aggregates"]:
                left = _aggregate_value(staging, table["name"], tenant, 0, aggregate)
                right = _aggregate_value(staging, table["name"], tenant, 1, aggregate)
                mismatch_count += int(left != right)
        report.append({"table": table["name"], "aggregate_mismatches": mismatch_count})
    return report


def _semantic_violations_disk(
    staging: sqlite3.Connection,
    rules: Sequence[Mapping[str, Any]],
    side: int,
) -> dict[str, int]:
    result: dict[str, int] = {}
    for rule in rules:
        if rule["kind"] == "unique_per_tenant":
            value = _json_value("item", rule["column"])
            query = f"""
                SELECT COALESCE(SUM(duplicates), 0) FROM (
                  SELECT COUNT(*) - 1 AS duplicates
                  FROM reconciliation_rows AS item
                  WHERE item.side = ? AND item.table_name = ?
                  GROUP BY item.tenant, {value}
                  HAVING COUNT(*) > 1
                )
            """
            violations = staging.execute(query, (side, rule["table"])).fetchone()[0]
        else:
            parent_id = _json_value("parent", rule["parent_id_column"])
            owner = _json_value("parent", rule["owner_column"])
            member_parent = _json_value("member", rule["membership_parent_column"])
            member_user = _json_value("member", rule["membership_user_column"])
            role = _json_value("member", rule["role_column"])
            state = _json_value("member", rule["state_column"])
            query = f"""
                SELECT COUNT(*) FROM reconciliation_rows AS parent
                WHERE parent.side = ? AND parent.table_name = ?
                  AND NOT EXISTS (
                    SELECT 1 FROM reconciliation_rows AS member
                    WHERE member.side = ? AND member.table_name = ?
                      AND member.tenant = parent.tenant
                      AND {member_parent} IS {parent_id}
                      AND {member_user} IS {owner}
                      AND {role} IS ? AND {state} IS ?
                  )
            """
            violations = staging.execute(
                query,
                (
                    side,
                    rule["parent_table"],
                    side,
                    rule["membership_table"],
                    rule["required_role"],
                    rule["required_state"],
                ),
            ).fetchone()[0]
        result[rule["name"]] = violations
    return result


def reconcile_bundle(target: pathlib.Path, bundle: pathlib.Path) -> Mapping[str, Any]:
    manifest = load_manifest(bundle)
    try:
        scratch_parent = bundle.parent.resolve(strict=True)
        scratch_metadata = scratch_parent.lstat()
    except OSError as error:
        raise EtlError("reconciliation scratch parent is unavailable") from error
    if (
        not stat.S_ISDIR(scratch_metadata.st_mode)
        or stat.S_ISLNK(scratch_metadata.st_mode)
        or scratch_metadata.st_uid != os.getuid()
        or scratch_metadata.st_mode & 0o077
    ):
        raise EtlError("reconciliation scratch parent must be owner-private")
    try:
        target_uri = target.resolve(strict=True).as_uri() + "?mode=ro"
    except OSError as error:
        raise EtlError("reconciliation target is unavailable") from error
    with tempfile.TemporaryDirectory(
        prefix=".frame-etl-reconcile-", dir=scratch_parent
    ) as raw:
        staging_path = pathlib.Path(raw)
        staging_path.chmod(0o700)
        staging = _reconciliation_database(staging_path / "reconciliation.sqlite")
        target_connection = sqlite3.connect(target_uri, uri=True)
        target_connection.row_factory = sqlite3.Row
        target_connection.execute("PRAGMA foreign_keys = ON")
        target_connection.execute("PRAGMA query_only = ON")
        target_connection.execute("BEGIN")
        try:
            _stage_reconciliation(staging, target_connection, bundle, manifest)
            sections: list[dict[str, Any]] = []
            mismatch_total = 0
            scopes = staging.execute(
                "SELECT table_name, tenant FROM reconciliation_scopes "
                "GROUP BY table_name, tenant ORDER BY table_name, tenant"
            )
            for table_name, tenant in scopes:
                if len(sections) >= MAX_MANIFEST_SECTIONS:
                    raise EtlError("reconciliation exceeds the supported tenant-section count")
                expected_rows, actual_rows = staging.execute(
                    "SELECT SUM(CASE WHEN side = 0 THEN 1 ELSE 0 END), "
                    "SUM(CASE WHEN side = 1 THEN 1 ELSE 0 END) "
                    "FROM reconciliation_rows WHERE table_name = ? AND tenant = ?",
                    (table_name, tenant),
                ).fetchone()
                expected_rows = expected_rows or 0
                actual_rows = actual_rows or 0
                missing = staging.execute(
                    "SELECT COUNT(*) FROM reconciliation_rows AS expected "
                    "WHERE expected.side = 0 AND expected.table_name = ? AND expected.tenant = ? "
                    "AND NOT EXISTS (SELECT 1 FROM reconciliation_rows AS actual "
                    "WHERE actual.side = 1 AND actual.table_name = expected.table_name "
                    "AND actual.tenant = expected.tenant AND actual.logical_key = expected.logical_key)",
                    (table_name, tenant),
                ).fetchone()[0]
                extra = staging.execute(
                    "SELECT COUNT(*) FROM reconciliation_rows AS actual "
                    "WHERE actual.side = 1 AND actual.table_name = ? AND actual.tenant = ? "
                    "AND NOT EXISTS (SELECT 1 FROM reconciliation_rows AS expected "
                    "WHERE expected.side = 0 AND expected.table_name = actual.table_name "
                    "AND expected.tenant = actual.tenant AND expected.logical_key = actual.logical_key)",
                    (table_name, tenant),
                ).fetchone()[0]
                changed = staging.execute(
                    "SELECT COUNT(*) FROM reconciliation_rows AS expected "
                    "JOIN reconciliation_rows AS actual ON actual.side = 1 "
                    "AND actual.table_name = expected.table_name AND actual.tenant = expected.tenant "
                    "AND actual.logical_key = expected.logical_key "
                    "WHERE expected.side = 0 AND expected.table_name = ? AND expected.tenant = ? "
                    "AND expected.row_hash != actual.row_hash",
                    (table_name, tenant),
                ).fetchone()[0]
                mismatch_total += missing + extra + changed
                sections.append(
                    {
                        "table": table_name,
                        "tenant_digest": tenant_digest(tenant),
                        "expected_rows": expected_rows,
                        "actual_rows": actual_rows,
                        "missing_primary_keys": missing,
                        "extra_primary_keys": extra,
                        "field_hash_mismatches": changed,
                    }
                )
            expected_relationship = _relationship_violations_disk(staging, manifest["tables"], 0)
            actual_relationship = _relationship_violations_disk(staging, manifest["tables"], 1)
            sqlite_foreign_keys = sum(1 for _row in target_connection.execute("PRAGMA foreign_key_check"))
            aggregate_report = _aggregate_mismatches_disk(staging, manifest["tables"])
            expected_semantics = _semantic_violations_disk(staging, manifest["semantic_rules"], 0)
            actual_semantics = _semantic_violations_disk(staging, manifest["semantic_rules"], 1)
        finally:
            target_connection.close()
            staging.close()
    aggregate_mismatches = sum(section["aggregate_mismatches"] for section in aggregate_report)
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
