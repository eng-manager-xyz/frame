#!/usr/bin/env python3
"""Export a deterministic MySQL snapshot into the Frame ETL bundle contract.

The exporter runs one MySQL client session, captures a conservative GTID boundary
before opening a read-only repeatable-read snapshot, and streams canonical NDJSON
into :mod:`etl`. Credentials are accepted only through an owner-private MySQL
defaults file; they never appear in arguments, reports, or exception messages.

The GTID boundary intentionally precedes the snapshot. Incremental replay may see
records that are already present in the snapshot, but the Frame target is
idempotent and therefore prefers safe duplication over an unobservable gap.
"""

from __future__ import annotations

import argparse
import base64
import ctypes
import errno
import fcntl
import hashlib
import json
import os
import pathlib
import re
import selectors
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from typing import BinaryIO, Mapping, Sequence

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import etl


SCHEMA_VERSION = 1
BEGIN_MARKER = "__FRAME_MYSQL_SNAPSHOT_V1_BEGIN__"
END_MARKER = "__FRAME_MYSQL_SNAPSHOT_V1_END__"
BINDING_MARKER = "__FRAME_MYSQL_SOURCE_BINDING_V1__"
MAX_ROW_BYTES = 16 * 1024 * 1024
MAX_GTID_BYTES = 4 * 1024 * 1024
MAX_STDERR_BYTES = 1024 * 1024
MAX_SOURCE_BYTES = 2 * 1024 * 1024 * 1024
MAX_DEFAULTS_BYTES = 64 * 1024
MAX_BINDING_BYTES = 64 * 1024 * 1024
MINIMUM_MYSQL_VERSION = "8.0.26"
DEFAULT_TIMEOUT_SECONDS = 14_400
ALLOWED_DEFAULT_OPTIONS = frozenset(
    {
        "host",
        "port",
        "user",
        "password",
        "password1",
        "password2",
        "password3",
        "database",
        "ssl-ca",
        "ssl-capath",
        "ssl-cert",
        "ssl-key",
    }
)
REQUIRED_DEFAULT_OPTIONS = frozenset({"host", "user", "database"})
SOURCE_BINDING_FIELDS = (
    "database_sha256",
    "server_uuid_sha256",
    "server_version_sha256",
    "tables_sha256",
    "columns_sha256",
    "indexes_sha256",
    "constraints_sha256",
)
MYSQL_IDENTIFIER = re.compile(r"^[A-Za-z][A-Za-z0-9_]{0,63}$")


class SnapshotError(Exception):
    """A stable operator-safe failure without source or credential values."""


@dataclass(frozen=True)
class SnapshotCapture:
    row_count: int
    source_bytes: int
    started_at_ms: int
    completed_at_ms: int
    gtid_set: str
    query_sha256: str


def _mysql_identifier(value: object) -> str:
    if not isinstance(value, str) or not MYSQL_IDENTIFIER.fullmatch(value):
        raise SnapshotError("snapshot plan contains an invalid MySQL identifier")
    return f"`{value}`"


def _mysql_string(value: str) -> str:
    if not etl.IDENTIFIER.fullmatch(value):
        raise SnapshotError("snapshot plan contains an invalid static label")
    return f"'{value}'"


def _source_table(raw_table: Mapping[str, object]) -> str:
    value = raw_table.get("source_table", raw_table.get("name"))
    _mysql_identifier(value)
    return str(value)


def _source_tenant_column(table: etl.Table) -> str:
    if table.tenant_column is None:
        if table.tenant_source is None:
            raise SnapshotError("snapshot plan lacks an explicit source tenant")
        return table.tenant_source
    matches = [
        column.input_sources()[0]
        for column in table.columns
        if column.target == table.tenant_column and len(column.input_sources()) == 1
    ]
    if len(matches) != 1:
        raise SnapshotError("snapshot plan must map exactly one source tenant column")
    return matches[0]


def _source_primary_key(
    table: etl.Table, raw_table: Mapping[str, object]
) -> tuple[str, ...]:
    configured = raw_table.get("source_primary_key")
    if configured is not None:
        if (
            not isinstance(configured, list)
            or not configured
            or len(configured) > 16
            or len(configured) != len(set(configured))
            or not all(isinstance(item, str) and etl.IDENTIFIER.fullmatch(item) for item in configured)
        ):
            raise SnapshotError("snapshot plan source primary key is invalid")
        available = {
            source for column in table.columns for source in column.input_sources()
        }
        if not set(configured).issubset(available):
            raise SnapshotError("snapshot plan source primary key is invalid")
        return tuple(configured)
    result: list[str] = []
    for target in table.primary_key:
        matches = [
            column.input_sources()[0]
            for column in table.columns
            if column.target == target and len(column.input_sources()) == 1
        ]
        if len(matches) != 1:
            raise SnapshotError("snapshot plan must map every source primary-key column exactly once")
        result.append(matches[0])
    return tuple(result)


def _snapshot_expectations(plan: etl.Plan) -> dict[str, str]:
    raw = plan.raw.get("mysql_snapshot")
    if not isinstance(raw, dict) or set(raw) != set(SOURCE_BINDING_FIELDS):
        raise SnapshotError("snapshot plan lacks the exact MySQL source binding")
    result: dict[str, str] = {}
    for field in SOURCE_BINDING_FIELDS:
        value = raw.get(field)
        if not isinstance(value, str) or not etl.SHA256.fullmatch(value):
            raise SnapshotError("snapshot plan contains an invalid MySQL source binding")
        result[field] = value
    return result


def _text_expression(identifier: str, *, timestamp: bool) -> str:
    if timestamp:
        return (
            f"CASE WHEN {identifier} IS NULL THEN NULL ELSE "
            f"DATE_FORMAT({identifier}, '%Y-%m-%dT%H:%i:%s.%fZ') END"
        )
    return f"CAST({identifier} AS CHAR CHARACTER SET utf8mb4)"


def _projection_expression(raw_table: Mapping[str, object], logical_source: str) -> str:
    fields = raw_table.get("source_fields", {})
    if not isinstance(fields, dict):
        raise SnapshotError("snapshot source projection fields are invalid")
    raw_field = fields.get(logical_source)
    if raw_field is None:
        alias = raw_table.get("source_alias", "source")
        column = logical_source
    elif isinstance(raw_field, dict) and set(raw_field) == {"alias", "column"}:
        alias = raw_field["alias"]
        column = raw_field["column"]
    elif isinstance(raw_field, dict) and raw_field.get("kind") == "row_number":
        if set(raw_field) != {"kind", "partition_by", "order_by"}:
            raise SnapshotError("snapshot row-number projection is invalid")
        partition = raw_field["partition_by"]
        order = raw_field["order_by"]
        if (
            not isinstance(partition, list)
            or not 1 <= len(partition) <= 8
            or not isinstance(order, list)
            or not 1 <= len(order) <= 8
        ):
            raise SnapshotError("snapshot row-number projection is invalid")

        def references(items: object) -> str:
            assert isinstance(items, list)
            expressions: list[str] = []
            for item in items:
                if not isinstance(item, dict) or set(item) != {"alias", "column"}:
                    raise SnapshotError("snapshot row-number projection is invalid")
                expressions.append(
                    f"{_mysql_identifier(item['alias'])}.{_mysql_identifier(item['column'])}"
                )
            return ", ".join(expressions)

        return (
            "ROW_NUMBER() OVER (PARTITION BY "
            f"{references(partition)} ORDER BY {references(order)})"
        )
    else:
        raise SnapshotError("snapshot source projection field is invalid")
    return f"{_mysql_identifier(alias)}.{_mysql_identifier(column)}"


def _projection_from(raw_table: Mapping[str, object]) -> str:
    source = _mysql_identifier(_source_table(raw_table))
    source_alias = raw_table.get("source_alias", "source")
    base = f"{source} AS {_mysql_identifier(source_alias)}"
    joins = raw_table.get("joins", [])
    if not isinstance(joins, list) or len(joins) > 16:
        raise SnapshotError("snapshot source projection joins are invalid")
    parts = [base]
    known_aliases = {str(source_alias)}
    for join in joins:
        if (
            not isinstance(join, dict)
            or set(join) != {"table", "alias", "on"}
            or not isinstance(join.get("on"), list)
            or not join["on"]
            or len(join["on"]) > 8
        ):
            raise SnapshotError("snapshot source projection join is invalid")
        table = join["table"]
        alias = join["alias"]
        _mysql_identifier(table)
        _mysql_identifier(alias)
        if alias in known_aliases:
            raise SnapshotError("snapshot source projection repeats an alias")
        predicates: list[str] = []
        predicate_aliases: set[str] = set()
        for predicate in join["on"]:
            if not isinstance(predicate, dict) or set(predicate) != {"left", "right"}:
                raise SnapshotError("snapshot source projection predicate is invalid")
            sides: list[str] = []
            for side in (predicate["left"], predicate["right"]):
                if not isinstance(side, dict) or set(side) != {"alias", "column"}:
                    raise SnapshotError("snapshot source projection predicate is invalid")
                predicate_aliases.add(str(side["alias"]))
                sides.append(
                    f"{_mysql_identifier(side['alias'])}.{_mysql_identifier(side['column'])}"
                )
            predicates.append(f"{sides[0]} = {sides[1]}")
        if (
            not predicate_aliases.issubset(known_aliases | {str(alias)})
            or str(alias) not in predicate_aliases
            or not predicate_aliases & known_aliases
        ):
            raise SnapshotError("snapshot source projection predicate is invalid")
        parts.append(
            f"JOIN {_mysql_identifier(table)} AS {_mysql_identifier(alias)} ON "
            + " AND ".join(predicates)
        )
        known_aliases.add(str(alias))
    fields = raw_table.get("source_fields", {})
    if not isinstance(fields, dict):
        raise SnapshotError("snapshot source projection references an unknown alias")
    for value in fields.values():
        if not isinstance(value, dict):
            raise SnapshotError("snapshot source projection references an unknown alias")
        if value.get("kind") == "row_number":
            collections = (value.get("partition_by"), value.get("order_by"))
            aliases = {
                item.get("alias")
                for collection in collections
                if isinstance(collection, list)
                for item in collection
                if isinstance(item, dict)
            }
        else:
            aliases = {value.get("alias")}
        if not aliases or not aliases.issubset(known_aliases):
            raise SnapshotError("snapshot source projection references an unknown alias")
    return " ".join(parts)


def _table_query(table: etl.Table, raw_table: Mapping[str, object]) -> str:
    tenant_source = _source_tenant_column(table)
    source_columns = list(
        dict.fromkeys(source for column in table.columns for source in column.input_sources())
    )
    row_items = ", ".join(
        f"{_mysql_string(source)}, {_text_expression(_projection_expression(raw_table, source), timestamp=any(column.transform == 'timestamp_ms' and source in column.input_sources() for column in table.columns))}"
        for source in source_columns
    )
    order_columns = (tenant_source, *_source_primary_key(table, raw_table))
    order = ", ".join(_projection_expression(raw_table, column) for column in order_columns)
    tenant_expression = _projection_expression(raw_table, tenant_source)
    return (
        "SELECT JSON_OBJECT("
        f"'table', {_mysql_string(table.name)}, "
        f"'tenant', CAST({tenant_expression} AS CHAR CHARACTER SET utf8mb4), "
        f"'row', JSON_OBJECT({row_items})) "
        f"FROM {_projection_from(raw_table)} ORDER BY {order};"
    )


def _source_binding_statements(table_list: str) -> list[str]:
    return [
        "SET @frame_binding_warnings = 0;",
        (
            "SET @frame_tables_payload = (SELECT GROUP_CONCAT(JSON_ARRAY("
            "table_name, engine, table_collation, row_format, create_options) "
            "ORDER BY table_name SEPARATOR '|') FROM information_schema.tables "
            f"WHERE table_schema = DATABASE() AND table_name IN ({table_list}));"
        ),
        "SET @frame_binding_warnings = @frame_binding_warnings + @@warning_count;",
        (
            "SET @frame_columns_payload = (SELECT GROUP_CONCAT(JSON_ARRAY(table_name, "
            "column_name, ordinal_position, column_default, is_nullable, data_type, "
            "character_maximum_length, numeric_precision, numeric_scale, datetime_precision, "
            "character_set_name, collation_name, column_type, column_key, extra, "
            "generation_expression) ORDER BY table_name, ordinal_position SEPARATOR '|') "
            "FROM information_schema.columns WHERE table_schema = DATABASE() "
            f"AND table_name IN ({table_list}));"
        ),
        "SET @frame_binding_warnings = @frame_binding_warnings + @@warning_count;",
        (
            "SET @frame_indexes_payload = (SELECT GROUP_CONCAT(JSON_ARRAY(table_name, "
            "index_name, non_unique, seq_in_index, column_name, collation, sub_part, packed, "
            "nullable, index_type, comment, index_comment, expression, is_visible) "
            "ORDER BY table_name, index_name, seq_in_index SEPARATOR '|') "
            "FROM information_schema.statistics WHERE table_schema = DATABASE() "
            f"AND table_name IN ({table_list}));"
        ),
        "SET @frame_binding_warnings = @frame_binding_warnings + @@warning_count;",
        (
            "SET @frame_constraints_payload = (SELECT GROUP_CONCAT(JSON_ARRAY("
            "tc.table_name, tc.constraint_name, tc.constraint_type, tc.enforced, "
            "kcu.ordinal_position, kcu.column_name, kcu.referenced_table_name, "
            "kcu.referenced_column_name, rc.match_option, rc.update_rule, rc.delete_rule, "
            "cc.check_clause) ORDER BY tc.table_name, tc.constraint_name, "
            "COALESCE(kcu.ordinal_position, 0) SEPARATOR '|') "
            "FROM information_schema.table_constraints AS tc "
            "LEFT JOIN information_schema.key_column_usage AS kcu "
            "ON kcu.constraint_schema = tc.constraint_schema "
            "AND kcu.table_name = tc.table_name AND kcu.constraint_name = tc.constraint_name "
            "LEFT JOIN information_schema.referential_constraints AS rc "
            "ON rc.constraint_schema = tc.constraint_schema "
            "AND rc.table_name = tc.table_name AND rc.constraint_name = tc.constraint_name "
            "LEFT JOIN information_schema.check_constraints AS cc "
            "ON cc.constraint_schema = tc.constraint_schema "
            "AND cc.constraint_name = tc.constraint_name "
            "WHERE tc.constraint_schema = DATABASE() "
            f"AND tc.table_name IN ({table_list}));"
        ),
        "SET @frame_binding_warnings = @frame_binding_warnings + @@warning_count;",
    ]


def _server_compatible_expression() -> str:
    return (
        "(CAST(SUBSTRING_INDEX(VERSION(), '.', 1) AS UNSIGNED) > 8 OR "
        "(CAST(SUBSTRING_INDEX(VERSION(), '.', 1) AS UNSIGNED) = 8 AND "
        "(CAST(SUBSTRING_INDEX(SUBSTRING_INDEX(VERSION(), '.', 2), '.', -1) AS UNSIGNED) > 0 OR "
        "(CAST(SUBSTRING_INDEX(SUBSTRING_INDEX(VERSION(), '.', 2), '.', -1) AS UNSIGNED) = 0 AND "
        "CAST(SUBSTRING_INDEX(SUBSTRING_INDEX(VERSION(), '.', 3), '.', -1) AS UNSIGNED) >= 26))))"
    )


def _planned_source_tables(plan: etl.Plan) -> tuple[list[Mapping[str, object]], list[str]]:
    raw_tables = plan.raw.get("tables")
    if not isinstance(raw_tables, list) or len(raw_tables) != len(plan.tables):
        raise SnapshotError("snapshot plan table mapping is inconsistent")
    validated: list[Mapping[str, object]] = []
    source_tables: list[str] = []
    for raw_table in raw_tables:
        if not isinstance(raw_table, dict):
            raise SnapshotError("snapshot plan table mapping is invalid")
        validated.append(raw_table)
        source_tables.append(_source_table(raw_table))
        joins = raw_table.get("joins", [])
        if not isinstance(joins, list):
            raise SnapshotError("snapshot source projection joins are invalid")
        source_tables.extend(str(join.get("table")) for join in joins if isinstance(join, dict))
    return validated, list(dict.fromkeys(source_tables))


def source_binding_sql(plan: etl.Plan) -> str:
    _raw_tables, source_tables = _planned_source_tables(plan)
    table_list = ", ".join(_mysql_string(table) for table in source_tables)
    statements = [
        "SET SESSION time_zone = '+00:00';",
        "SET SESSION TRANSACTION ISOLATION LEVEL REPEATABLE READ;",
        f"SET SESSION group_concat_max_len = {MAX_BINDING_BYTES};",
        "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY;",
    ]
    statements.extend(
        f"SELECT 1 FROM {_mysql_identifier(table)} LIMIT 0;" for table in source_tables
    )
    statements.extend(_source_binding_statements(table_list))
    statements.append(
        "SELECT CONCAT("
        f"'{BINDING_MARKER}', CHAR(9), {_server_compatible_expression()}, CHAR(9), "
        "@@GLOBAL.gtid_mode, CHAR(9), @@GLOBAL.enforce_gtid_consistency, CHAR(9), "
        "@@GLOBAL.log_bin, CHAR(9), @@GLOBAL.log_replica_updates, CHAR(9), "
        "@@GLOBAL.binlog_format, CHAR(9), @@GLOBAL.binlog_row_image, CHAR(9), "
        "@@GLOBAL.binlog_row_value_options, CHAR(9), @frame_binding_warnings, CHAR(9), "
        "(SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() "
        f"AND table_name IN ({table_list}) AND table_type = 'BASE TABLE' "
        "AND engine = 'InnoDB'), CHAR(9), SHA2(COALESCE(DATABASE(), ''), 256), CHAR(9), "
        "SHA2(@@GLOBAL.server_uuid, 256), CHAR(9), SHA2(VERSION(), 256), CHAR(9), "
        "SHA2(COALESCE(@frame_tables_payload, ''), 256), CHAR(9), "
        "SHA2(COALESCE(@frame_columns_payload, ''), 256), CHAR(9), "
        "SHA2(COALESCE(@frame_indexes_payload, ''), 256), CHAR(9), "
        "SHA2(COALESCE(@frame_constraints_payload, ''), 256));"
    )
    statements.append("COMMIT;")
    return "\n".join(statements) + "\n"
def snapshot_sql(plan: etl.Plan) -> str:
    raw_tables, source_tables = _planned_source_tables(plan)
    table_list = ", ".join(_mysql_string(table) for table in source_tables)
    statements = [
        "SET SESSION time_zone = '+00:00';",
        "SET SESSION TRANSACTION ISOLATION LEVEL REPEATABLE READ;",
        f"SET SESSION group_concat_max_len = {MAX_BINDING_BYTES};",
        "SET @frame_gtid_before_snapshot = @@GLOBAL.gtid_executed;",
        "SET @frame_snapshot_started_ms = CAST(UNIX_TIMESTAMP(UTC_TIMESTAMP(3)) * 1000 AS UNSIGNED);",
        "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY;",
    ]
    statements.extend(
        f"SELECT 1 FROM {_mysql_identifier(table)} LIMIT 0;" for table in source_tables
    )
    statements.extend(_source_binding_statements(table_list))
    statements.append(
        (
            "SELECT CONCAT("
            f"'{BEGIN_MARKER}', CHAR(9), @frame_snapshot_started_ms, CHAR(9), "
            "REPLACE(TO_BASE64(@frame_gtid_before_snapshot), CHAR(10), ''), CHAR(9), "
            f"{_server_compatible_expression()}, CHAR(9), "
            "@@GLOBAL.gtid_mode, CHAR(9), @@GLOBAL.enforce_gtid_consistency, CHAR(9), "
            "@@GLOBAL.log_bin, CHAR(9), @@GLOBAL.log_replica_updates, CHAR(9), "
            "@@GLOBAL.binlog_format, CHAR(9), "
            "@@GLOBAL.binlog_row_image, CHAR(9), @@GLOBAL.binlog_row_value_options, CHAR(9), "
            "@frame_binding_warnings, CHAR(9), "
            "(SELECT COUNT(*) FROM information_schema.tables "
            f"WHERE table_schema = DATABASE() AND table_name IN ({table_list}) "
            "AND table_type = 'BASE TABLE' AND engine = 'InnoDB'), CHAR(9), "
            "SHA2(COALESCE(DATABASE(), ''), 256), CHAR(9), "
            "SHA2(@@GLOBAL.server_uuid, 256), CHAR(9), SHA2(VERSION(), 256), CHAR(9), "
            "SHA2(COALESCE(@frame_tables_payload, ''), 256), CHAR(9), "
            "SHA2(COALESCE(@frame_columns_payload, ''), 256), CHAR(9), "
            "SHA2(COALESCE(@frame_indexes_payload, ''), 256), CHAR(9), "
            "SHA2(COALESCE(@frame_constraints_payload, ''), 256));"
        )
    )
    for table, raw_table in zip(plan.tables, raw_tables, strict=True):
        if not isinstance(raw_table, dict):
            raise SnapshotError("snapshot plan table mapping is invalid")
        statements.append(_table_query(table, raw_table))
    statements.extend(
        [
            "SET @frame_snapshot_completed_ms = CAST(UNIX_TIMESTAMP(UTC_TIMESTAMP(3)) * 1000 AS UNSIGNED);",
            f"SELECT CONCAT('{END_MARKER}', CHAR(9), @frame_snapshot_completed_ms);",
            "COMMIT;",
        ]
    )
    return "\n".join(statements) + "\n"


def _private_defaults_file(path: pathlib.Path) -> bytes:
    descriptor: int | None = None
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_NOFOLLOW", 0),
        )
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_mode & 0o077
        ):
            raise SnapshotError("MySQL defaults file must be an owner-private regular file")
        with os.fdopen(descriptor, "rb", closefd=True) as handle:
            descriptor = None
            raw_content = handle.read(MAX_DEFAULTS_BYTES + 1)
    except OSError as error:
        raise SnapshotError("MySQL defaults file is unavailable") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)
    if not 1 <= len(raw_content) <= MAX_DEFAULTS_BYTES:
        raise SnapshotError("MySQL defaults file is outside its supported size bound")
    try:
        content = raw_content.decode("utf-8")
    except UnicodeDecodeError as error:
        raise SnapshotError("MySQL defaults file is unreadable") from error
    section: str | None = None
    options: set[str] = set()
    for raw_line in content.splitlines():
        line = raw_line.strip()
        if not line or line.startswith(("#", ";")):
            continue
        if line.startswith("!"):
            raise SnapshotError("MySQL defaults file may not include other files")
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1].strip().casefold()
            if section != "client":
                raise SnapshotError("MySQL defaults file may contain only the client group")
            continue
        if section != "client" or "=" not in line:
            raise SnapshotError("MySQL defaults file contains an unsupported entry")
        name, _value = line.split("=", maxsplit=1)
        name = name.strip().casefold().replace("_", "-")
        if name not in ALLOWED_DEFAULT_OPTIONS or name in options:
            raise SnapshotError("MySQL defaults file contains an unsupported or repeated option")
        options.add(name)
    if not REQUIRED_DEFAULT_OPTIONS.issubset(options) or not {"ssl-ca", "ssl-capath"} & options:
        raise SnapshotError("MySQL defaults file lacks required connection or CA options")
    return raw_content


def _publication_lock(path: pathlib.Path) -> int:
    try:
        descriptor = os.open(
            path,
            os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_mode & 0o077
        ):
            raise SnapshotError("snapshot publication lock file is unsafe")
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return descriptor
    except (OSError, SnapshotError) as error:
        if "descriptor" in locals():
            os.close(descriptor)
        if isinstance(error, SnapshotError):
            raise
        raise SnapshotError("snapshot bundle publication is already reserved") from error


def _rename_noreplace(source: pathlib.Path, destination: pathlib.Path) -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    source_bytes = os.fsencode(source)
    destination_bytes = os.fsencode(destination)
    if sys.platform == "darwin" and hasattr(libc, "renamex_np"):
        operation = libc.renamex_np
        operation.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
        operation.restype = ctypes.c_int
        result = operation(source_bytes, destination_bytes, 0x00000004)
    elif sys.platform.startswith("linux") and hasattr(libc, "renameat2"):
        operation = libc.renameat2
        operation.argtypes = [
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_uint,
        ]
        operation.restype = ctypes.c_int
        result = operation(-100, source_bytes, -100, destination_bytes, 0x00000001)
    else:
        raise SnapshotError("atomic no-replace publication is unavailable on this platform")
    if result == 0:
        return
    error_number = ctypes.get_errno()
    if error_number in (errno.EEXIST, errno.ENOTEMPTY):
        raise SnapshotError("bundle path appeared during immutable publication")
    raise SnapshotError("atomic snapshot publication failed safely") from OSError(error_number, os.strerror(error_number))


def _mysql_executable(value: str) -> pathlib.Path:
    resolved = shutil.which(value)
    if resolved is None:
        raise SnapshotError("approved MySQL client executable was not found")
    path = pathlib.Path(resolved).resolve(strict=True)
    try:
        metadata = path.stat()
    except OSError as error:
        raise SnapshotError("approved MySQL client executable is unavailable") from error
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_mode & 0o111 == 0
        or metadata.st_uid not in (0, os.getuid())
        or metadata.st_mode & 0o022
    ):
        raise SnapshotError("approved MySQL client executable is invalid")
    return path


def _mysql_arguments(executable: pathlib.Path, defaults: pathlib.Path) -> list[str]:
    return [
        str(executable),
        f"--defaults-file={defaults}",
        "--protocol=TCP",
        "--ssl-mode=VERIFY_IDENTITY",
        "--tls-version=TLSv1.2,TLSv1.3",
        "--connect-timeout=15",
        f"--max-allowed-packet={MAX_ROW_BYTES}",
        "--default-character-set=utf8mb4",
        "--batch",
        "--raw",
        "--silent",
        "--skip-column-names",
        "--binary-mode=1",
        "--local-infile=0",
        "--skip-reconnect",
        "--quick",
    ]


def _child_environment(login_path: pathlib.Path) -> dict[str, str]:
    environment = {
        key: os.environ[key]
        for key in ("LANG", "LC_ALL")
        if key in os.environ
    }
    environment["TZ"] = "UTC"
    environment["MYSQL_TEST_LOGIN_FILE"] = str(login_path)
    return environment


def _owner_private_directory(path: pathlib.Path, description: str) -> pathlib.Path:
    try:
        resolved = path.resolve(strict=True)
        metadata = resolved.lstat()
    except OSError as error:
        raise SnapshotError(f"{description} is unavailable") from error
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_ISLNK(metadata.st_mode)
        or metadata.st_uid != os.getuid()
        or metadata.st_mode & 0o077
    ):
        raise SnapshotError(f"{description} must be an owner-private directory")
    return resolved


def _run_bounded_marker_query(
    *,
    query: str,
    defaults_file: pathlib.Path,
    mysql_bin: str,
    scratch_directory: pathlib.Path,
    timeout_seconds: int,
) -> str:
    if not 1 <= timeout_seconds <= DEFAULT_TIMEOUT_SECONDS:
        raise SnapshotError("source fingerprint timeout is outside the supported range")
    scratch = _owner_private_directory(scratch_directory, "fingerprint scratch directory")
    defaults_payload = _private_defaults_file(defaults_file)
    executable = _mysql_executable(mysql_bin)
    with tempfile.TemporaryDirectory(prefix=".frame-mysql-fingerprint.", dir=scratch) as raw:
        temporary = pathlib.Path(raw)
        temporary.chmod(0o700)
        staged_defaults = temporary / "client.cnf"
        query_path = temporary / "fingerprint.sql"
        login_path = temporary / "absent-login-path"
        etl.immutable_private_write(staged_defaults, defaults_payload)
        etl.immutable_private_write(query_path, query.encode("utf-8"))
        process: subprocess.Popen[bytes] | None = None
        output = bytearray()
        stderr_bytes = 0
        started = time.monotonic()
        try:
            with query_path.open("rb") as query_handle:
                process = subprocess.Popen(
                    _mysql_arguments(executable, staged_defaults),
                    stdin=query_handle,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    close_fds=True,
                    env=_child_environment(login_path),
                )
                assert process.stdout is not None and process.stderr is not None
                selector = selectors.DefaultSelector()
                selector.register(process.stdout, selectors.EVENT_READ, "stdout")
                selector.register(process.stderr, selectors.EVENT_READ, "stderr")
                while selector.get_map():
                    if time.monotonic() - started > timeout_seconds:
                        raise SnapshotError("source fingerprint exceeded its bounded execution time")
                    events = selector.select(timeout=1)
                    if not events and process.poll() is not None:
                        events = [
                            (registered, selectors.EVENT_READ)
                            for registered in selector.get_map().values()
                        ]
                    for key, _mask in events:
                        chunk = os.read(key.fd, 64 * 1024)
                        if not chunk:
                            selector.unregister(key.fileobj)
                            continue
                        if key.data == "stderr":
                            stderr_bytes += len(chunk)
                            if stderr_bytes > MAX_STDERR_BYTES:
                                raise SnapshotError(
                                    "source fingerprint diagnostic output exceeded its safe bound"
                                )
                        else:
                            output.extend(chunk)
                            if len(output) > MAX_ROW_BYTES:
                                raise SnapshotError("source fingerprint output exceeded its safe bound")
                exit_code = process.wait(timeout=5)
                if exit_code != 0:
                    raise SnapshotError("source fingerprint client failed safely")
        except BaseException:
            if process is not None:
                _terminate(process)
            raise
        if not output.endswith(b"\n") or output.count(b"\n") != 1:
            raise SnapshotError("source fingerprint returned an invalid response shape")
        try:
            return output[:-1].removesuffix(b"\r").decode("ascii")
        except UnicodeDecodeError as error:
            raise SnapshotError("source fingerprint returned a non-ASCII response") from error


def fingerprint_mysql_source(
    *,
    plan_path: pathlib.Path,
    defaults_file: pathlib.Path,
    mysql_bin: str,
    scratch_directory: pathlib.Path,
    timeout_seconds: int,
) -> Mapping[str, object]:
    plan = etl.load_plan(plan_path)
    line = _run_bounded_marker_query(
        query=source_binding_sql(plan),
        defaults_file=defaults_file,
        mysql_bin=mysql_bin,
        scratch_directory=scratch_directory,
        timeout_seconds=timeout_seconds,
    )
    fields = line.split("\t")
    if len(fields) != 18 or fields[0] != BINDING_MARKER:
        raise SnapshotError("source fingerprint marker is invalid")
    if fields[1:9] != ["1", "ON", "ON", "1", "1", "ROW", "FULL", ""]:
        raise SnapshotError("MySQL source does not satisfy the reviewed GTID/binlog policy")
    if fields[9] != "0":
        raise SnapshotError("MySQL source fingerprint exceeded its complete metadata bound")
    source_table_count = len(_planned_source_tables(plan)[1])
    if _parse_millis(fields[10], "fingerprint InnoDB table count") != source_table_count:
        raise SnapshotError("MySQL fingerprint includes a missing, view, or non-InnoDB table")
    binding = dict(zip(SOURCE_BINDING_FIELDS, fields[11:18], strict=True))
    if not all(etl.SHA256.fullmatch(value) for value in binding.values()):
        raise SnapshotError("MySQL source fingerprint returned an invalid digest")
    return {
        "schema_version": SCHEMA_VERSION,
        "minimum_mysql_version": MINIMUM_MYSQL_VERSION,
        "preflight_policy": "mysql_gtid_row_full_innodb_v2",
        "mysql_snapshot": binding,
        "contains_source_values": False,
        "production_evidence": False,
    }


def _parse_millis(value: str, label: str) -> int:
    try:
        parsed = int(value)
    except ValueError as error:
        raise SnapshotError(f"MySQL snapshot {label} marker is invalid") from error
    if not 0 <= parsed <= etl.MAX_WIRE_INTEGER:
        raise SnapshotError(f"MySQL snapshot {label} marker is invalid")
    return parsed


def _parse_begin(
    line: str,
    expected_innodb_tables: int,
    expected_source: Mapping[str, str],
) -> tuple[int, str]:
    fields = line.split("\t")
    if len(fields) != 20 or fields[0] != BEGIN_MARKER:
        raise SnapshotError("MySQL snapshot begin marker is invalid")
    started_at_ms = _parse_millis(fields[1], "begin")
    try:
        encoded = fields[2].encode("ascii")
        if len(encoded) > (MAX_GTID_BYTES * 4 // 3) + 8:
            raise SnapshotError("MySQL GTID boundary exceeds the supported size")
        gtid = base64.b64decode(encoded, validate=True).decode("ascii")
    except (UnicodeEncodeError, UnicodeDecodeError, ValueError) as error:
        raise SnapshotError("MySQL GTID boundary is invalid") from error
    if not gtid or len(gtid.encode("ascii")) > MAX_GTID_BYTES:
        raise SnapshotError("MySQL GTID boundary is unavailable or too large")
    if not all(character.isalnum() or character in "-_ :,\n\r\t" for character in gtid):
        raise SnapshotError("MySQL GTID boundary contains unsupported characters")
    prerequisites = fields[3:11]
    if prerequisites != ["1", "ON", "ON", "1", "1", "ROW", "FULL", ""]:
        raise SnapshotError("MySQL source does not satisfy the reviewed GTID/binlog policy")
    if fields[11] != "0":
        raise SnapshotError("MySQL source binding exceeded its complete metadata bound")
    if _parse_millis(fields[12], "InnoDB table count") != expected_innodb_tables:
        raise SnapshotError("MySQL snapshot includes a missing, view, or non-InnoDB source table")
    actual_source = dict(zip(SOURCE_BINDING_FIELDS, fields[13:20], strict=True))
    if actual_source != expected_source:
        raise SnapshotError("MySQL source identity or schema differs from the pinned plan")
    return started_at_ms, gtid


def _parse_end(line: str) -> int:
    fields = line.split("\t")
    if len(fields) != 2 or fields[0] != END_MARKER:
        raise SnapshotError("MySQL snapshot end marker is invalid")
    return _parse_millis(fields[1], "end")


def _canonical_source_line(line: bytes, tables: set[str]) -> bytes:
    if len(line) > MAX_ROW_BYTES:
        raise SnapshotError("MySQL snapshot row exceeds the supported size")
    try:
        decoded = line.decode("utf-8")
        envelope = etl.strict_json(decoded)
    except (UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
        raise SnapshotError("MySQL snapshot returned an invalid row envelope") from error
    if (
        not isinstance(envelope, dict)
        or set(envelope) != {"table", "tenant", "row"}
        or envelope.get("table") not in tables
        or not isinstance(envelope.get("tenant"), str)
        or not isinstance(envelope.get("row"), dict)
    ):
        raise SnapshotError("MySQL snapshot returned an invalid row envelope")
    return f"{etl.canonical(envelope)}\n".encode("utf-8")


def _terminate(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def capture_snapshot(
    *,
    plan: etl.Plan,
    defaults_file: pathlib.Path,
    mysql_bin: str,
    source_path: pathlib.Path,
    timeout_seconds: int,
) -> SnapshotCapture:
    if not 1 <= timeout_seconds <= DEFAULT_TIMEOUT_SECONDS:
        raise SnapshotError("snapshot timeout is outside the supported range")
    defaults_payload = _private_defaults_file(defaults_file)
    executable = _mysql_executable(mysql_bin)
    expected_source = _snapshot_expectations(plan)
    query = snapshot_sql(plan)
    query_sha256 = hashlib.sha256(query.encode("utf-8")).hexdigest()
    private_defaults = source_path.parent / ".frame-mysql-snapshot-client.cnf"
    if private_defaults.exists():
        raise SnapshotError("private MySQL defaults staging path is already in use")
    etl.immutable_private_write(private_defaults, defaults_payload)
    args = _mysql_arguments(executable, private_defaults)
    etl.private_directory(source_path.parent)
    query_path = source_path.parent / ".frame-mysql-snapshot-query.sql"
    if query_path.exists():
        private_defaults.unlink(missing_ok=True)
        raise SnapshotError("private MySQL snapshot query path is already in use")
    try:
        etl.immutable_private_write(query_path, query.encode("utf-8"))
    except BaseException:
        private_defaults.unlink(missing_ok=True)
        raise
    try:
        login_descriptor, login_name = tempfile.mkstemp(
            prefix=".frame-empty-mysql-login-path.", dir=source_path.parent
        )
    except BaseException:
        query_path.unlink(missing_ok=True)
        private_defaults.unlink(missing_ok=True)
        raise
    os.close(login_descriptor)
    pathlib.Path(login_name).unlink()
    child_environment = _child_environment(pathlib.Path(login_name))
    try:
        source_descriptor = os.open(
            source_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600
        )
    except BaseException:
        pathlib.Path(login_name).unlink(missing_ok=True)
        query_path.unlink(missing_ok=True)
        private_defaults.unlink(missing_ok=True)
        raise
    started_monotonic = time.monotonic()
    process: subprocess.Popen[bytes] | None = None
    row_count = 0
    source_bytes = 0
    started_at_ms: int | None = None
    completed_at_ms: int | None = None
    gtid_set: str | None = None
    pending = bytearray()
    stderr_bytes = 0
    tables = {table.name for table in plan.tables}
    try:
        with os.fdopen(source_descriptor, "wb") as source_handle, query_path.open(
            "rb"
        ) as query_handle:
            process = subprocess.Popen(
                args,
                stdin=query_handle,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                close_fds=True,
                env=child_environment,
            )
            assert process.stdout is not None and process.stderr is not None
            selector = selectors.DefaultSelector()
            selector.register(process.stdout, selectors.EVENT_READ, "stdout")
            selector.register(process.stderr, selectors.EVENT_READ, "stderr")
            while selector.get_map():
                if time.monotonic() - started_monotonic > timeout_seconds:
                    raise SnapshotError("MySQL snapshot exceeded its bounded execution time")
                events = selector.select(timeout=1)
                if not events and process.poll() is not None:
                    events = [
                        (registered, selectors.EVENT_READ)
                        for registered in selector.get_map().values()
                    ]
                for key, _mask in events:
                    chunk = os.read(key.fd, 64 * 1024)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    if key.data == "stderr":
                        stderr_bytes += len(chunk)
                        if stderr_bytes > MAX_STDERR_BYTES:
                            raise SnapshotError(
                                "MySQL snapshot diagnostic output exceeded its safe bound"
                            )
                        continue
                    pending.extend(chunk)
                    if len(pending) > MAX_ROW_BYTES and b"\n" not in pending:
                        raise SnapshotError("MySQL snapshot row exceeds the supported size")
                    while b"\n" in pending:
                        raw_line, _, remainder = pending.partition(b"\n")
                        pending = bytearray(remainder)
                        if raw_line.endswith(b"\r"):
                            raw_line = raw_line[:-1]
                        if not raw_line:
                            raise SnapshotError("MySQL snapshot returned an empty record")
                        try:
                            marker = raw_line.decode("ascii")
                        except UnicodeDecodeError:
                            marker = ""
                        if started_at_ms is None:
                            source_table_count = len(_planned_source_tables(plan)[1])
                            started_at_ms, gtid_set = _parse_begin(
                                marker, source_table_count, expected_source
                            )
                            continue
                        if marker.startswith(END_MARKER):
                            if completed_at_ms is not None:
                                raise SnapshotError("MySQL snapshot returned duplicate end markers")
                            completed_at_ms = _parse_end(marker)
                            continue
                        if completed_at_ms is not None:
                            raise SnapshotError("MySQL snapshot returned rows after its end marker")
                        canonical = _canonical_source_line(bytes(raw_line), tables)
                        source_bytes += len(canonical)
                        if source_bytes > MAX_SOURCE_BYTES:
                            raise SnapshotError("MySQL snapshot exceeds the supported source size")
                        source_handle.write(canonical)
                        row_count += 1
            if pending:
                raise SnapshotError("MySQL snapshot ended with a truncated record")
            exit_code = process.wait(timeout=5)
            source_handle.flush()
            os.fsync(source_handle.fileno())
            if exit_code != 0:
                raise SnapshotError("MySQL snapshot client failed safely")
    except BaseException:
        if process is not None:
            _terminate(process)
        source_path.unlink(missing_ok=True)
        raise
    finally:
        query_path.unlink(missing_ok=True)
        private_defaults.unlink(missing_ok=True)
        pathlib.Path(login_name).unlink(missing_ok=True)
    if started_at_ms is None or completed_at_ms is None or gtid_set is None:
        source_path.unlink(missing_ok=True)
        raise SnapshotError("MySQL snapshot did not return complete boundary markers")
    if completed_at_ms < started_at_ms:
        source_path.unlink(missing_ok=True)
        raise SnapshotError("MySQL snapshot returned a reversed boundary")
    if not (
        plan.window_start_ms <= started_at_ms
        and completed_at_ms <= plan.window_end_ms
    ):
        source_path.unlink(missing_ok=True)
        raise SnapshotError("MySQL snapshot fell outside the approved plan window")
    return SnapshotCapture(
        row_count=row_count,
        source_bytes=source_bytes,
        started_at_ms=started_at_ms,
        completed_at_ms=completed_at_ms,
        gtid_set=gtid_set,
        query_sha256=query_sha256,
    )


def export_mysql_snapshot(
    *,
    plan_path: pathlib.Path,
    defaults_file: pathlib.Path,
    mysql_bin: str,
    bundle: pathlib.Path,
    chunk_rows: int,
    timeout_seconds: int,
) -> Mapping[str, object]:
    plan = etl.load_plan(plan_path)
    source_binding = _snapshot_expectations(plan)
    if os.path.lexists(bundle):
        raise SnapshotError("bundle path already exists; immutable snapshots are never overwritten")
    parent = bundle.parent.resolve()
    try:
        metadata = parent.lstat()
    except OSError as error:
        raise SnapshotError("snapshot bundle parent is unavailable") from error
    if (
        not stat.S_ISDIR(metadata.st_mode)
        or stat.S_ISLNK(metadata.st_mode)
        or metadata.st_uid != os.getuid()
        or metadata.st_mode & 0o077
    ):
        raise SnapshotError("snapshot bundle parent must be an owner-private directory")
    lock_path = parent / f".{bundle.name}.publish.lock"
    lock_descriptor = _publication_lock(lock_path)
    try:
        if os.path.lexists(bundle):
            raise SnapshotError("bundle path already exists; immutable snapshots are never overwritten")
        with tempfile.TemporaryDirectory(prefix=".frame-mysql-snapshot.", dir=parent) as raw:
            temporary = pathlib.Path(raw)
            temporary.chmod(0o700)
            source = temporary / "source.ndjson"
            capture = capture_snapshot(
                plan=plan,
                defaults_file=defaults_file,
                mysql_bin=mysql_bin,
                source_path=source,
                timeout_seconds=timeout_seconds,
            )
            source_hasher = hashlib.sha256()
            with source.open("rb") as source_handle:
                for chunk in iter(lambda: source_handle.read(1024 * 1024), b""):
                    source_hasher.update(chunk)
            source_sha256 = source_hasher.hexdigest()
            plan_sha256 = etl.sha256_json(plan.raw)
            gtid_sha256 = hashlib.sha256(capture.gtid_set.encode("ascii")).hexdigest()
            source_binding_sha256 = etl.sha256_json(source_binding)
            staged_bundle = temporary / "bundle"
            manifest = etl.export_bundle(
                source,
                plan,
                staged_bundle,
                chunk_rows,
            )
            manifest_core_sha256 = etl.manifest_core_sha256(manifest)
            protected_boundary = {
                "schema_version": SCHEMA_VERSION,
                "run_id": plan.run_id,
                "snapshot_started_at_ms": capture.started_at_ms,
                "snapshot_completed_at_ms": capture.completed_at_ms,
                "gtid_before_snapshot": capture.gtid_set,
                "gtid_sha256": gtid_sha256,
                "query_sha256": capture.query_sha256,
                "source_binding_sha256": source_binding_sha256,
                "manifest_core_sha256": manifest_core_sha256,
                "source_sha256": source_sha256,
                "plan_sha256": plan_sha256,
                "preflight_policy": "mysql_gtid_row_full_innodb_v2",
            }
            public_proof = {
                "schema_version": SCHEMA_VERSION,
                "run_id": plan.run_id,
                "snapshot_started_at_ms": capture.started_at_ms,
                "snapshot_completed_at_ms": capture.completed_at_ms,
                "captured_row_count": capture.row_count,
                "source_bytes": capture.source_bytes,
                "gtid_sha256": gtid_sha256,
                "query_sha256": capture.query_sha256,
                "source_binding_sha256": source_binding_sha256,
                "manifest_core_sha256": manifest_core_sha256,
                "source_sha256": source_sha256,
                "plan_sha256": plan_sha256,
                "preflight_policy": "mysql_gtid_row_full_innodb_v2",
                "contains_source_values": False,
                "production_evidence": False,
            }
            boundary_payload = f"{etl.canonical(protected_boundary)}\n".encode("utf-8")
            proof_payload = f"{etl.canonical(public_proof)}\n".encode("utf-8")
            source_attestation = {
                "kind": etl.MYSQL_SNAPSHOT_ATTESTATION_KIND,
                "boundary_path": etl.MYSQL_SNAPSHOT_BOUNDARY,
                "boundary_sha256": hashlib.sha256(boundary_payload).hexdigest(),
                "proof_path": etl.MYSQL_SNAPSHOT_PROOF,
                "proof_sha256": hashlib.sha256(proof_payload).hexdigest(),
                "query_sha256": capture.query_sha256,
                "gtid_sha256": gtid_sha256,
                "source_binding_sha256": source_binding_sha256,
                "manifest_core_sha256": manifest_core_sha256,
            }
            manifest = etl.attach_source_attestation(
                staged_bundle, manifest, source_attestation
            )
            etl.immutable_private_write(
                staged_bundle / etl.MYSQL_SNAPSHOT_BOUNDARY,
                boundary_payload,
            )
            etl.immutable_private_write(
                staged_bundle / etl.MYSQL_SNAPSHOT_PROOF,
                proof_payload,
            )
            verified = etl.load_manifest(staged_bundle)
            if verified != manifest:
                raise SnapshotError("staged MySQL snapshot manifest changed during verification")
            for table in manifest["tables"]:
                for tenant in table["tenants"]:
                    for chunk in tenant["chunks"]:
                        etl.read_chunk(staged_bundle, chunk)
            for directory in sorted(
                (path for path in staged_bundle.rglob("*") if path.is_dir()),
                key=lambda path: len(path.parts),
                reverse=True,
            ):
                descriptor = os.open(directory, os.O_RDONLY)
                try:
                    os.fsync(descriptor)
                finally:
                    os.close(descriptor)
            descriptor = os.open(staged_bundle, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
            _rename_noreplace(staged_bundle, bundle)
            parent_descriptor = os.open(parent, os.O_RDONLY)
            try:
                os.fsync(parent_descriptor)
            finally:
                os.close(parent_descriptor)
        return {"manifest": manifest, "proof": public_proof}
    finally:
        fcntl.flock(lock_descriptor, fcntl.LOCK_UN)
        os.close(lock_descriptor)


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    root.add_argument(
        "--fingerprint-source",
        action="store_true",
        help="emit a value-free source binding for plan review instead of exporting rows",
    )
    root.add_argument("--plan", type=pathlib.Path, required=True)
    root.add_argument("--defaults-file", type=pathlib.Path, required=True)
    root.add_argument("--mysql-bin", default="mysql")
    root.add_argument("--bundle", type=pathlib.Path)
    root.add_argument("--scratch-directory", type=pathlib.Path)
    root.add_argument("--chunk-rows", type=int, default=1_000)
    root.add_argument("--timeout-seconds", type=int, default=DEFAULT_TIMEOUT_SECONDS)
    return root


def main(arguments: Sequence[str] | None = None) -> int:
    args = parser().parse_args(arguments)
    try:
        if args.fingerprint_source:
            if args.scratch_directory is None or args.bundle is not None:
                raise SnapshotError(
                    "source fingerprint requires --scratch-directory and forbids --bundle"
                )
            result = fingerprint_mysql_source(
                plan_path=args.plan,
                defaults_file=args.defaults_file,
                mysql_bin=args.mysql_bin,
                scratch_directory=args.scratch_directory,
                timeout_seconds=args.timeout_seconds,
            )
            print(etl.canonical(result))
            return 0
        if args.bundle is None:
            raise SnapshotError("snapshot export requires --bundle")
        result = export_mysql_snapshot(
            plan_path=args.plan,
            defaults_file=args.defaults_file,
            mysql_bin=args.mysql_bin,
            bundle=args.bundle,
            chunk_rows=args.chunk_rows,
            timeout_seconds=args.timeout_seconds,
        )
        proof = result["proof"]
        assert isinstance(proof, dict)
        print(
            f"captured {proof['captured_row_count']} rows in approved snapshot window; "
            f"boundary digest {proof['gtid_sha256']}"
        )
        manifest = result["manifest"]
        assert isinstance(manifest, dict)
        return 0 if manifest["reject_count"] == 0 else 3
    except (
        SnapshotError,
        etl.EtlError,
        OSError,
        sqlite3.Error,
        subprocess.SubprocessError,
    ) as error:
        print(f"MySQL snapshot failed safely: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
