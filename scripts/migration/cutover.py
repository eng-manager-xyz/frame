#!/usr/bin/env python3
"""Local shadow-read, change-capture, and authority-cutover rehearsal controls.

The state database is durable and owner-only. Reports contain only counts, bounded
labels, tenant digests, and result digests; captured payload values never reach stdout,
audit evidence, shadow reports, or lag metrics. The dual-write phase has one writer
(legacy) plus an asynchronous D1 mirror—it does not claim cross-database atomicity.
"""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
import pathlib
import re
import sqlite3
import stat
import sys
from dataclasses import dataclass
from typing import Any, Mapping, Sequence

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import etl


PHASE_WRITER = {
    "legacy_authoritative": "legacy",
    "shadow_read": "legacy",
    "dual_write": "legacy",
    "d1_authoritative": "d1",
    "rolled_back": "legacy",
    "finalized": "d1",
}
ALLOWED_TRANSITIONS = {
    "legacy_authoritative": {"shadow_read"},
    "shadow_read": {"dual_write"},
    "dual_write": {"d1_authoritative"},
    "d1_authoritative": {"rolled_back", "finalized"},
    "rolled_back": {"dual_write"},
    "finalized": set(),
}
MIRROR_PHASES = {"dual_write", "d1_authoritative", "rolled_back"}
SAFE_EVIDENCE_KEYS = {
    "clean_reconciliation",
    "dead_letter_events",
    "d1_fenced",
    "legacy_caught_up",
    "legacy_fenced",
    "observation_window_complete",
    "pending_events",
    "reconciliation_sha256",
    "rollback_rehearsed",
    "shadow_mismatches",
    "shadow_observation_ready",
}
COUNT_EVIDENCE_KEYS = {"dead_letter_events", "pending_events", "shadow_mismatches"}
MAX_CONTROL_FILE_BYTES = 1024 * 1024
MAX_CAPTURE_FILE_BYTES = 64 * 1024 * 1024
MAX_CAPTURE_EVENT_BYTES = 16 * 1024 * 1024
MAX_CAPTURE_EVENTS = 100_000
MAX_SHADOW_ROWS = 100_000
MAX_SHADOW_FIELDS = 256
SIGNAL_KINDS = {
    "authority_contention",
    "replay_write_failure",
    "replay_lost_ack",
}
CUTOVER_DOMAIN = re.compile(r"^[a-z][a-z0-9_-]{0,63}$")


class CutoverError(Exception):
    """An operator-safe failure which must never contain payload values."""


class InjectedReplayInterruption(Exception):
    """A fault after target commit but before source event acknowledgement."""


@dataclass(frozen=True)
class CutoverConfig:
    plan: etl.Plan
    operator_digests: frozenset[str]
    tenant_digests: frozenset[str]
    domains: frozenset[str]
    shadow_queries: Mapping[str, Mapping[str, Any]]
    shadow_window_ms: int
    minimum_shadow_observations: int
    max_pending_lag_ms: int
    max_shadow_mismatches: int
    max_dead_letter_events: int
    max_contention_events: int
    maintenance_windows: tuple[tuple[int, int], ...]


def safe_ms(value: Any, description: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= etl.MAX_WIRE_INTEGER:
        raise CutoverError(f"{description} timestamp is invalid")
    return value


def load_config(path: pathlib.Path) -> CutoverConfig:
    plan = etl.load_plan(path)
    raw = plan.raw.get("cutover")
    if not isinstance(raw, dict) or set(raw) != {
        "authorized_operator_digests",
        "authorized_tenant_digests",
        "domains",
        "maintenance_windows",
        "slo",
        "shadow_queries",
    }:
        raise CutoverError("cutover configuration is invalid")
    operators = raw["authorized_operator_digests"]
    tenants = raw["authorized_tenant_digests"]
    domains = raw["domains"]
    queries = raw["shadow_queries"]
    if (
        not isinstance(operators, list)
        or not operators
        or not all(isinstance(item, str) and etl.SHA256.fullmatch(item) for item in operators)
    ):
        raise CutoverError("authorized operator digests are invalid")
    if (
        not isinstance(domains, list)
        or not domains
        or not all(isinstance(item, str) and CUTOVER_DOMAIN.fullmatch(item) for item in domains)
    ):
        raise CutoverError("cutover domains are invalid")
    if (
        not isinstance(tenants, list)
        or not tenants
        or len(tenants) > 10_000
        or not all(isinstance(item, str) and etl.SHA256.fullmatch(item) for item in tenants)
    ):
        raise CutoverError("authorized cutover tenants are invalid")
    if not isinstance(queries, dict) or not queries or len(queries) > 128:
        raise CutoverError("shadow query configuration is invalid")
    for name, query in queries.items():
        if not isinstance(name, str) or not etl.SAFE_LABEL.fullmatch(name) or not isinstance(query, dict):
            raise CutoverError("shadow query configuration is invalid")
        if set(query) != {
            "order_by",
            "casefold_fields",
            "ignored_fields",
            "timestamp_fields",
            "timestamp_precision_ms",
        }:
            raise CutoverError("shadow query normalization is invalid")
        field_groups = [
            query["order_by"],
            query["casefold_fields"],
            query["ignored_fields"],
            query["timestamp_fields"],
        ]
        if not all(
            isinstance(group, list)
            and len(group) <= MAX_SHADOW_FIELDS
            and len(group) == len(set(group))
            and all(isinstance(field, str) for field in group)
            for group in field_groups
        ):
            raise CutoverError("shadow query field lists are invalid")
        for field in [field for group in field_groups for field in group]:
            etl.quote(field)
        if not query["order_by"]:
            raise CutoverError("shadow query requires a deterministic ordering key")
        if set(query["ignored_fields"]) & (
            set(query["order_by"])
            | set(query["casefold_fields"])
            | set(query["timestamp_fields"])
        ):
            raise CutoverError("ignored shadow fields cannot also be normalized")
        precision = query["timestamp_precision_ms"]
        if isinstance(precision, bool) or not isinstance(precision, int) or not 1 <= precision <= 60_000:
            raise CutoverError("shadow timestamp precision is invalid")
    if (
        len(operators) != len(set(operators))
        or len(tenants) != len(set(tenants))
        or len(domains) != len(set(domains))
    ):
        raise CutoverError("cutover authorization scopes contain duplicates")
    slo = raw["slo"]
    if not isinstance(slo, dict) or set(slo) != {
        "shadow_window_ms",
        "minimum_shadow_observations",
        "max_pending_lag_ms",
        "max_shadow_mismatches",
        "max_dead_letter_events",
        "max_contention_events",
    }:
        raise CutoverError("cutover SLO configuration is invalid")
    for key, value in slo.items():
        if (
            isinstance(value, bool)
            or not isinstance(value, int)
            or not 0 <= value <= etl.MAX_WIRE_INTEGER
        ):
            raise CutoverError("cutover SLO configuration is invalid")
    if (
        not 1 <= slo["shadow_window_ms"] <= 30 * 24 * 60 * 60 * 1000
        or not 1 <= slo["minimum_shadow_observations"] <= 1_000_000
        or not 1 <= slo["max_pending_lag_ms"] <= 30 * 24 * 60 * 60 * 1000
    ):
        raise CutoverError("cutover SLO configuration is outside its safe bound")
    windows = raw["maintenance_windows"]
    if not isinstance(windows, list) or not windows or len(windows) > 1_000:
        raise CutoverError("cutover maintenance windows are invalid")
    parsed_windows: list[tuple[int, int]] = []
    previous_end = -1
    for window in windows:
        if not isinstance(window, dict) or set(window) != {"start_ms", "end_ms"}:
            raise CutoverError("cutover maintenance windows are invalid")
        start = safe_ms(window["start_ms"], "maintenance window start")
        end = safe_ms(window["end_ms"], "maintenance window end")
        if start > end or start <= previous_end:
            raise CutoverError("cutover maintenance windows overlap or are reversed")
        parsed_windows.append((start, end))
        previous_end = end
    return CutoverConfig(
        plan,
        frozenset(operators),
        frozenset(tenants),
        frozenset(domains),
        queries,
        slo["shadow_window_ms"],
        slo["minimum_shadow_observations"],
        slo["max_pending_lag_ms"],
        slo["max_shadow_mismatches"],
        slo["max_dead_letter_events"],
        slo["max_contention_events"],
        tuple(parsed_windows),
    )


def open_state(path: pathlib.Path) -> sqlite3.Connection:
    try:
        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        parent = path.parent.resolve(strict=True)
        parent_metadata = parent.lstat()
    except OSError as error:
        raise CutoverError("cutover state parent is unavailable") from error
    if (
        not stat.S_ISDIR(parent_metadata.st_mode)
        or stat.S_ISLNK(parent_metadata.st_mode)
        or parent_metadata.st_uid != os.getuid()
        or parent_metadata.st_mode & 0o077
    ):
        raise CutoverError("cutover state parent must be owner-private")
    state_path = parent / path.name
    descriptor: int | None = None
    try:
        flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open(state_path, flags, 0o600)
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_mode & 0o077
        ):
            raise CutoverError("cutover state file must be owner-private")
    except OSError as error:
        raise CutoverError("cutover state file is unavailable") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)
    connection = sqlite3.connect(state_path, timeout=5)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA foreign_keys = ON")
    connection.execute("PRAGMA secure_delete = ON")
    connection.execute("PRAGMA journal_mode = WAL")
    existing_state = connection.execute(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'authority_state'"
    ).fetchone()
    if existing_state is not None:
        state_columns = {
            row["name"] for row in connection.execute("PRAGMA table_info(authority_state)")
        }
        legacy_columns = {
            "tenant_digest",
            "domain",
            "phase",
            "writer",
            "mirror_enabled",
            "replay_paused",
            "epoch",
            "audit_head",
            "updated_at_ms",
        }
        if not legacy_columns.issubset(state_columns):
            connection.close()
            raise CutoverError("cutover state schema is incompatible")
        if "rollback_ready" not in state_columns:
            connection.execute(
                """ALTER TABLE authority_state ADD COLUMN rollback_ready INTEGER
                   NOT NULL DEFAULT 0 CHECK(rollback_ready IN (0, 1))"""
            )
            connection.commit()
        if "phase_started_at_ms" not in state_columns:
            if connection.execute("SELECT 1 FROM authority_state LIMIT 1").fetchone() is not None:
                connection.close()
                raise CutoverError("cutover state lacks a trustworthy phase-window boundary")
            connection.execute(
                """ALTER TABLE authority_state ADD COLUMN phase_started_at_ms INTEGER
                   NOT NULL DEFAULT 0 CHECK(phase_started_at_ms BETWEEN 0 AND 9007199254740991)"""
            )
            connection.commit()
    existing_events = connection.execute(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'captured_events'"
    ).fetchone()
    if existing_events is not None:
        event_info = connection.execute("PRAGMA table_info(captured_events)").fetchall()
        event_columns = {row["name"] for row in event_info}
        event_primary_key = [
            row["name"]
            for row in sorted(event_info, key=lambda row: row["pk"])
            if row["pk"]
        ]
        if (
            "source_authority" not in event_columns
            or event_primary_key != ["tenant_digest", "domain", "event_id"]
        ):
            connection.close()
            raise CutoverError("cutover event schema is incompatible")
    connection.executescript(
        """
        CREATE TABLE IF NOT EXISTS authority_state (
          tenant_digest TEXT NOT NULL CHECK(length(tenant_digest) = 64),
          domain TEXT NOT NULL,
          phase TEXT NOT NULL CHECK(phase IN (
            'legacy_authoritative', 'shadow_read', 'dual_write',
            'd1_authoritative', 'rolled_back', 'finalized'
          )),
          writer TEXT NOT NULL CHECK(writer IN ('legacy', 'd1')),
          mirror_enabled INTEGER NOT NULL CHECK(mirror_enabled IN (0, 1)),
          replay_paused INTEGER NOT NULL CHECK(replay_paused IN (0, 1)),
          epoch INTEGER NOT NULL CHECK(epoch BETWEEN 0 AND 9007199254740991),
          audit_head TEXT NOT NULL CHECK(length(audit_head) = 64),
          rollback_ready INTEGER NOT NULL CHECK(rollback_ready IN (0, 1)),
          phase_started_at_ms INTEGER NOT NULL
            CHECK(phase_started_at_ms BETWEEN 0 AND 9007199254740991),
          updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms BETWEEN 0 AND 9007199254740991),
          CHECK(
            (phase IN ('legacy_authoritative', 'shadow_read', 'dual_write', 'rolled_back')
              AND writer = 'legacy')
            OR (phase IN ('d1_authoritative', 'finalized') AND writer = 'd1')
          ),
          CHECK(mirror_enabled = (phase IN ('dual_write', 'd1_authoritative', 'rolled_back'))),
          CHECK(rollback_ready = (phase = 'd1_authoritative')),
          PRIMARY KEY(tenant_digest, domain)
        );
        CREATE TABLE IF NOT EXISTS authority_audit (
          audit_hash TEXT PRIMARY KEY NOT NULL CHECK(length(audit_hash) = 64),
          previous_hash TEXT NOT NULL CHECK(length(previous_hash) = 64),
          tenant_digest TEXT NOT NULL,
          domain TEXT NOT NULL,
          action TEXT NOT NULL CHECK(action IN ('transition', 'pause', 'resume')),
          from_phase TEXT NOT NULL,
          to_phase TEXT NOT NULL,
          from_epoch INTEGER NOT NULL,
          to_epoch INTEGER NOT NULL CHECK(to_epoch = from_epoch + 1),
          operator_digest TEXT NOT NULL CHECK(length(operator_digest) = 64),
          evidence_digest TEXT NOT NULL CHECK(length(evidence_digest) = 64),
          occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms BETWEEN 0 AND 9007199254740991)
        );
        CREATE TABLE IF NOT EXISTS captured_events (
          event_id TEXT NOT NULL,
          tenant_digest TEXT NOT NULL,
          domain TEXT NOT NULL,
          sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 9007199254740991),
          authority_epoch INTEGER NOT NULL CHECK(authority_epoch BETWEEN 0 AND 9007199254740991),
          source_authority TEXT NOT NULL CHECK(source_authority IN ('legacy', 'd1')),
          event_digest TEXT NOT NULL CHECK(length(event_digest) = 64),
          payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
          state TEXT NOT NULL CHECK(state IN ('pending', 'applied', 'dead_letter')),
          reason_code TEXT CHECK(reason_code IS NULL OR length(reason_code) BETWEEN 1 AND 128),
          occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms BETWEEN 0 AND 9007199254740991),
          captured_at_ms INTEGER NOT NULL CHECK(captured_at_ms BETWEEN 0 AND 9007199254740991),
          applied_at_ms INTEGER CHECK(applied_at_ms IS NULL OR applied_at_ms BETWEEN 0 AND 9007199254740991),
          PRIMARY KEY(tenant_digest, domain, event_id),
          UNIQUE(tenant_digest, domain, sequence)
        );
        CREATE INDEX IF NOT EXISTS captured_events_replay_idx
          ON captured_events(tenant_digest, domain, state, sequence);
        CREATE TABLE IF NOT EXISTS shadow_results (
          observation_digest TEXT PRIMARY KEY NOT NULL CHECK(length(observation_digest) = 64),
          tenant_digest TEXT NOT NULL,
          domain TEXT NOT NULL,
          query_class TEXT NOT NULL,
          legacy_digest TEXT NOT NULL CHECK(length(legacy_digest) = 64),
          d1_digest TEXT NOT NULL CHECK(length(d1_digest) = 64),
          classification TEXT NOT NULL CHECK(classification IN (
            'match', 'ordering_only', 'semantic_mismatch', 'missing', 'error'
          )),
          normalizations_applied INTEGER NOT NULL CHECK(normalizations_applied IN (0, 1)),
          observed_at_ms INTEGER NOT NULL CHECK(observed_at_ms BETWEEN 0 AND 9007199254740991)
        );
        CREATE INDEX IF NOT EXISTS shadow_results_scope_time_idx
          ON shadow_results(tenant_digest, domain, observed_at_ms, query_class);
        CREATE TABLE IF NOT EXISTS operational_signals (
          tenant_digest TEXT NOT NULL,
          domain TEXT NOT NULL,
          kind TEXT NOT NULL CHECK(kind IN (
            'authority_contention', 'replay_write_failure', 'replay_lost_ack'
          )),
          count INTEGER NOT NULL CHECK(count BETWEEN 0 AND 9007199254740991),
          last_at_ms INTEGER NOT NULL CHECK(last_at_ms BETWEEN 0 AND 9007199254740991),
          PRIMARY KEY(tenant_digest, domain, kind)
        ) WITHOUT ROWID;
        CREATE TABLE IF NOT EXISTS operational_signal_events (
          signal_id INTEGER PRIMARY KEY AUTOINCREMENT,
          tenant_digest TEXT NOT NULL CHECK(length(tenant_digest) = 64),
          domain TEXT NOT NULL,
          kind TEXT NOT NULL CHECK(kind IN (
            'authority_contention', 'replay_write_failure', 'replay_lost_ack'
          )),
          occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms BETWEEN 0 AND 9007199254740991)
        );
        CREATE INDEX IF NOT EXISTS operational_signal_events_scope_time_idx
          ON operational_signal_events(tenant_digest, domain, occurred_at_ms, kind);
        CREATE TRIGGER IF NOT EXISTS authority_audit_immutable_update
        BEFORE UPDATE ON authority_audit BEGIN SELECT RAISE(ABORT, 'authority audit is immutable'); END;
        CREATE TRIGGER IF NOT EXISTS authority_audit_immutable_delete
        BEFORE DELETE ON authority_audit BEGIN SELECT RAISE(ABORT, 'authority audit is immutable'); END;
        CREATE TRIGGER IF NOT EXISTS authority_state_epoch_monotonic
        BEFORE UPDATE ON authority_state WHEN NEW.epoch <> OLD.epoch + 1
        BEGIN SELECT RAISE(ABORT, 'authority epoch must advance exactly once'); END;
        CREATE TRIGGER IF NOT EXISTS authority_state_final_is_terminal
        BEFORE UPDATE ON authority_state WHEN OLD.phase = 'finalized'
        BEGIN SELECT RAISE(ABORT, 'finalized authority cannot transition'); END;
        """
    )
    connection.commit()
    return connection


def _window_start(
    connection: sqlite3.Connection,
    config: CutoverConfig,
    scope: tuple[str, str],
    now_ms: int,
) -> int:
    row = connection.execute(
        """SELECT phase_started_at_ms FROM authority_state
           WHERE tenant_digest = ? AND domain = ?""",
        scope,
    ).fetchone()
    if row is None:
        raise CutoverError("authority scope has not been initialized")
    return max(0, now_ms - config.shadow_window_ms, row["phase_started_at_ms"])


def _operational_health(
    connection: sqlite3.Connection,
    config: CutoverConfig,
    scope: tuple[str, str],
    now_ms: int,
) -> Mapping[str, Any]:
    start_ms = _window_start(connection, config, scope, now_ms)
    counts = {
        row["kind"]: row["count"]
        for row in connection.execute(
            """SELECT kind, COUNT(*) AS count
               FROM operational_signal_events
               WHERE tenant_digest = ? AND domain = ?
                 AND occurred_at_ms BETWEEN ? AND ?
               GROUP BY kind""",
            (*scope, start_ms, now_ms),
        )
    }
    return {
        "window_start_ms": start_ms,
        "window_end_ms": now_ms,
        "authority_contention": counts.get("authority_contention", 0),
        "replay_write_failure": counts.get("replay_write_failure", 0),
        "replay_lost_ack": counts.get("replay_lost_ack", 0),
    }


def _scope(config: CutoverConfig, tenant: str, domain: str) -> tuple[str, str]:
    if not isinstance(tenant, str) or not etl.SAFE_LABEL.fullmatch(tenant):
        raise CutoverError("tenant scope is invalid")
    if domain not in config.domains:
        raise CutoverError("cutover domain is not authorized by the plan")
    tenant_hash = etl.tenant_digest(tenant)
    if tenant_hash not in config.tenant_digests:
        raise CutoverError("tenant is not authorized by the cutover plan")
    return tenant_hash, domain


def _in_maintenance_window(config: CutoverConfig, occurred_at_ms: int) -> bool:
    return any(start <= occurred_at_ms <= end for start, end in config.maintenance_windows)


def _require_maintenance_window(config: CutoverConfig, occurred_at_ms: int) -> None:
    if not _in_maintenance_window(config, occurred_at_ms):
        raise CutoverError("forward cutover control is outside an approved maintenance window")


def _record_signal(
    connection: sqlite3.Connection,
    scope: tuple[str, str],
    kind: str,
    occurred_at_ms: int,
) -> None:
    if kind not in SIGNAL_KINDS:
        raise CutoverError("operational signal kind is invalid")
    connection.execute(
        """INSERT INTO operational_signals(tenant_digest, domain, kind, count, last_at_ms)
           VALUES (?, ?, ?, 1, ?)
           ON CONFLICT(tenant_digest, domain, kind) DO UPDATE SET
             count = count + 1, last_at_ms = excluded.last_at_ms""",
        (*scope, kind, occurred_at_ms),
    )
    connection.execute(
        """INSERT INTO operational_signal_events(
             tenant_digest, domain, kind, occurred_at_ms
           ) VALUES (?, ?, ?, ?)""",
        (*scope, kind, occurred_at_ms),
    )


def _shadow_health(
    connection: sqlite3.Connection,
    config: CutoverConfig,
    scope: tuple[str, str],
    now_ms: int,
) -> Mapping[str, Any]:
    start_ms = _window_start(connection, config, scope, now_ms)
    rows = connection.execute(
        """SELECT query_class, COUNT(*) AS observations,
                  SUM(CASE WHEN classification IN ('semantic_mismatch', 'missing', 'error')
                           THEN 1 ELSE 0 END) AS mismatches
           FROM shadow_results
           WHERE tenant_digest = ? AND domain = ?
             AND observed_at_ms BETWEEN ? AND ?
           GROUP BY query_class""",
        (*scope, start_ms, now_ms),
    ).fetchall()
    by_query = {
        row["query_class"]: {
            "observations": row["observations"],
            "mismatches": row["mismatches"],
        }
        for row in rows
    }
    coverage = {
        query: by_query.get(query, {"observations": 0, "mismatches": 0})
        for query in sorted(config.shadow_queries)
    }
    complete = all(
        item["observations"] >= config.minimum_shadow_observations
        for item in coverage.values()
    )
    mismatches = sum(item["mismatches"] for item in coverage.values())
    return {
        "window_start_ms": start_ms,
        "window_end_ms": now_ms,
        "required_query_classes": len(coverage),
        "coverage_complete": complete,
        "mismatches": mismatches,
        "queries": coverage,
    }


def _state(connection: sqlite3.Connection, scope: tuple[str, str]) -> sqlite3.Row:
    row = connection.execute(
        "SELECT * FROM authority_state WHERE tenant_digest = ? AND domain = ?", scope
    ).fetchone()
    if row is None:
        raise CutoverError("authority scope has not been initialized")
    if (
        PHASE_WRITER.get(row["phase"]) != row["writer"]
        or bool(row["mirror_enabled"]) != (row["phase"] in MIRROR_PHASES)
        or bool(row["rollback_ready"]) != (row["phase"] == "d1_authoritative")
        or isinstance(row["epoch"], bool)
        or not isinstance(row["epoch"], int)
        or not 0 <= row["epoch"] <= etl.MAX_WIRE_INTEGER
        or not isinstance(row["updated_at_ms"], int)
        or not 0 <= row["updated_at_ms"] <= etl.MAX_WIRE_INTEGER
        or not isinstance(row["phase_started_at_ms"], int)
        or not 0 <= row["phase_started_at_ms"] <= row["updated_at_ms"]
    ):
        raise CutoverError("authority state violates the single-writer invariant")
    return row


def initialize_scope(
    state_path: pathlib.Path, config: CutoverConfig, tenant: str, domain: str, occurred_at_ms: int
) -> Mapping[str, Any]:
    scope = _scope(config, tenant, domain)
    safe_ms(occurred_at_ms, "initialization")
    connection = open_state(state_path)
    try:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute(
            """INSERT INTO authority_state(
                 tenant_digest, domain, phase, writer, mirror_enabled, replay_paused,
                 epoch, audit_head, rollback_ready, phase_started_at_ms, updated_at_ms
               ) VALUES (?, ?, 'legacy_authoritative', 'legacy', 0, 0, 0, ?, 0, ?, ?)""",
            (*scope, "0" * 64, occurred_at_ms, occurred_at_ms),
        )
        connection.commit()
    except BaseException:
        connection.rollback()
        raise
    finally:
        connection.close()
    return {"tenant_digest": scope[0], "domain": domain, "phase": "legacy_authoritative", "writer": "legacy", "epoch": 0}


def _authorized_operator(config: CutoverConfig, operator_file: pathlib.Path) -> str:
    descriptor: int | None = None
    try:
        descriptor = os.open(
            operator_file,
            os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_NOFOLLOW", 0),
        )
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_mode & 0o077
            or not 1 <= metadata.st_size <= 1024
        ):
            raise CutoverError("operator credential must be an owner-private regular file")
        with os.fdopen(descriptor, "rb", closefd=True) as handle:
            descriptor = None
            value = handle.read(1025)
    except OSError as error:
        raise CutoverError("operator credential file is unreadable") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)
    if not value or len(value) > 1024 or not value.rstrip(b"\r\n"):
        raise CutoverError("operator credential is invalid")
    digest = hashlib.sha256(value.rstrip(b"\r\n")).hexdigest()
    if not any(hmac.compare_digest(digest, allowed) for allowed in config.operator_digests):
        raise CutoverError("operator is not authorized for migration controls")
    return digest


def _load_evidence(path: pathlib.Path) -> Mapping[str, Any]:
    try:
        payload = etl.bounded_regular_bytes(path, MAX_CONTROL_FILE_BYTES, "transition evidence")
        raw = etl.strict_json(payload.decode("utf-8"))
    except (etl.EtlError, UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
        raise CutoverError("transition evidence is unreadable or invalid") from error
    if not isinstance(raw, dict) or set(raw) - SAFE_EVIDENCE_KEYS:
        raise CutoverError("transition evidence contains unsupported fields")
    for key, value in raw.items():
        if key == "reconciliation_sha256":
            if not isinstance(value, str) or not etl.SHA256.fullmatch(value):
                raise CutoverError("transition reconciliation digest is invalid")
        elif key in COUNT_EVIDENCE_KEYS:
            if (
                isinstance(value, bool)
                or not isinstance(value, int)
                or not 0 <= value <= etl.MAX_WIRE_INTEGER
            ):
                raise CutoverError("transition count evidence is invalid")
        elif not isinstance(value, bool):
            raise CutoverError("transition evidence must contain only booleans, counts, and digests")
    return raw


def _gate_transition(
    current: str,
    target: str,
    evidence: Mapping[str, Any],
    shadow_health: Mapping[str, Any],
    operational_health: Mapping[str, Any],
    pending_lag_ms: int,
    config: CutoverConfig,
) -> None:
    required: dict[str, Any]
    if target == "shadow_read":
        required = {"shadow_observation_ready": True}
    elif target == "dual_write":
        required = {"clean_reconciliation": True, "shadow_mismatches": 0}
    elif target == "d1_authoritative":
        required = {
            "clean_reconciliation": True,
            "legacy_fenced": True,
            "pending_events": 0,
            "dead_letter_events": 0,
            "shadow_mismatches": 0,
            "rollback_rehearsed": True,
        }
    elif target == "rolled_back":
        required = {
            "clean_reconciliation": True,
            "d1_fenced": True,
            "legacy_caught_up": True,
            "rollback_rehearsed": True,
            "pending_events": 0,
            "dead_letter_events": 0,
        }
    elif target == "finalized":
        required = {
            "clean_reconciliation": True,
            "pending_events": 0,
            "dead_letter_events": 0,
            "shadow_mismatches": 0,
            "observation_window_complete": True,
        }
    else:
        raise CutoverError("target authority phase is invalid")
    if target in {"d1_authoritative", "rolled_back", "finalized"}:
        reconciliation_digest = evidence.get("reconciliation_sha256")
        if not isinstance(reconciliation_digest, str) or not etl.SHA256.fullmatch(reconciliation_digest):
            raise CutoverError("transition requires a reconciliation evidence digest")
    if target in {"dual_write", "d1_authoritative", "finalized"}:
        if not shadow_health["coverage_complete"]:
            raise CutoverError("transition lacks latest-window charter-query shadow coverage")
        if shadow_health["mismatches"] != 0:
            raise CutoverError("transition latest shadow window contains unexplained mismatches")
        if pending_lag_ms > config.max_pending_lag_ms:
            raise CutoverError("transition exceeds the approved replay-lag SLO")
        if operational_health["replay_write_failure"] != 0:
            raise CutoverError("transition window contains a replay write failure")
        if operational_health["replay_lost_ack"] != 0:
            raise CutoverError("transition window contains an unresolved replay acknowledgement loss")
        if operational_health["authority_contention"] > config.max_contention_events:
            raise CutoverError("transition exceeds the approved authority-contention SLO")
    missing = [key for key, value in required.items() if evidence.get(key) != value]
    if missing:
        # Field names are fixed, non-sensitive reason codes.
        raise CutoverError("transition evidence gate failed: " + ",".join(sorted(missing)))
    if target == "d1_authoritative" and current != "dual_write":
        raise CutoverError("D1 authority requires the controlled capture phase")


def _append_audit(
    connection: sqlite3.Connection,
    row: sqlite3.Row,
    *,
    action: str,
    to_phase: str,
    operator_digest: str,
    evidence: Mapping[str, Any],
    occurred_at_ms: int,
    paused: int,
) -> Mapping[str, Any]:
    if row["epoch"] >= etl.MAX_WIRE_INTEGER:
        raise CutoverError("authority epoch is exhausted")
    next_epoch = row["epoch"] + 1
    record = {
        "previous_hash": row["audit_head"],
        "tenant_digest": row["tenant_digest"],
        "domain": row["domain"],
        "action": action,
        "from_phase": row["phase"],
        "to_phase": to_phase,
        "from_epoch": row["epoch"],
        "to_epoch": next_epoch,
        "operator_digest": operator_digest,
        "evidence_digest": etl.sha256_json(evidence),
        "occurred_at_ms": occurred_at_ms,
    }
    audit_hash = etl.sha256_json(record)
    connection.execute(
        """INSERT INTO authority_audit(
             audit_hash, previous_hash, tenant_digest, domain, action, from_phase, to_phase,
             from_epoch, to_epoch, operator_digest, evidence_digest, occurred_at_ms
           ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (
            audit_hash,
            record["previous_hash"],
            record["tenant_digest"],
            record["domain"],
            action,
            record["from_phase"],
            to_phase,
            record["from_epoch"],
            next_epoch,
            operator_digest,
            record["evidence_digest"],
            occurred_at_ms,
        ),
    )
    connection.execute(
        """UPDATE authority_state SET
             phase = ?, writer = ?, mirror_enabled = ?, replay_paused = ?, epoch = ?,
             audit_head = ?, rollback_ready = ?, phase_started_at_ms = ?, updated_at_ms = ?
           WHERE tenant_digest = ? AND domain = ? AND epoch = ?""",
        (
            to_phase,
            PHASE_WRITER[to_phase],
            int(to_phase in MIRROR_PHASES),
            paused,
            next_epoch,
            audit_hash,
            int(to_phase == "d1_authoritative"),
            occurred_at_ms if action == "transition" else row["phase_started_at_ms"],
            occurred_at_ms,
            row["tenant_digest"],
            row["domain"],
            row["epoch"],
        ),
    )
    if connection.execute("SELECT changes()").fetchone()[0] != 1:
        raise CutoverError("authority epoch changed concurrently")
    return {
        "tenant_digest": row["tenant_digest"],
        "domain": row["domain"],
        "phase": to_phase,
        "writer": PHASE_WRITER[to_phase],
        "epoch": next_epoch,
        "audit_hash": audit_hash,
    }


def transition(
    state_path: pathlib.Path,
    config: CutoverConfig,
    tenant: str,
    domain: str,
    target_phase: str,
    expected_epoch: int,
    operator_file: pathlib.Path,
    evidence_path: pathlib.Path,
    occurred_at_ms: int,
) -> Mapping[str, Any]:
    scope = _scope(config, tenant, domain)
    safe_ms(occurred_at_ms, "transition")
    if isinstance(expected_epoch, bool) or not isinstance(expected_epoch, int) or not 0 <= expected_epoch <= etl.MAX_WIRE_INTEGER:
        raise CutoverError("authority epoch precondition is invalid")
    if target_phase != "rolled_back":
        _require_maintenance_window(config, occurred_at_ms)
    operator = _authorized_operator(config, operator_file)
    evidence = _load_evidence(evidence_path)
    connection = open_state(state_path)
    try:
        connection.execute("BEGIN IMMEDIATE")
        row = _state(connection, scope)
        if occurred_at_ms < row["updated_at_ms"]:
            raise CutoverError("transition time precedes the current authority state")
        if row["epoch"] != expected_epoch:
            _record_signal(connection, scope, "authority_contention", occurred_at_ms)
            connection.commit()
            raise CutoverError("authority epoch precondition failed")
        if target_phase not in ALLOWED_TRANSITIONS[row["phase"]]:
            raise CutoverError("authority transition is not allowed")
        live_event_counts = {
            item["state"]: item["count"]
            for item in connection.execute(
                """SELECT state, COUNT(*) AS count FROM captured_events
                   WHERE tenant_digest = ? AND domain = ? GROUP BY state""",
                scope,
            )
        }
        oldest_pending = connection.execute(
            """SELECT MIN(occurred_at_ms) AS value FROM captured_events
               WHERE tenant_digest = ? AND domain = ? AND state = 'pending'""",
            scope,
        ).fetchone()["value"]
        pending_lag_ms = (
            0 if oldest_pending is None else max(0, occurred_at_ms - oldest_pending)
        )
        shadow_health = _shadow_health(connection, config, scope, occurred_at_ms)
        operational_health = _operational_health(connection, config, scope, occurred_at_ms)
        live_values = {
            "pending_events": live_event_counts.get("pending", 0),
            "dead_letter_events": live_event_counts.get("dead_letter", 0),
            "shadow_mismatches": shadow_health["mismatches"],
        }
        for key, value in live_values.items():
            if key in evidence and evidence[key] != value:
                raise CutoverError(f"transition evidence disagrees with durable {key}")
        _gate_transition(
            row["phase"],
            target_phase,
            evidence,
            shadow_health,
            operational_health,
            pending_lag_ms,
            config,
        )
        result = _append_audit(
            connection,
            row,
            action="transition",
            to_phase=target_phase,
            operator_digest=operator,
            evidence=evidence,
            occurred_at_ms=occurred_at_ms,
            paused=0,
        )
        connection.commit()
        return result
    except BaseException:
        connection.rollback()
        raise
    finally:
        connection.close()


def replay_control(
    state_path: pathlib.Path,
    config: CutoverConfig,
    tenant: str,
    domain: str,
    action: str,
    expected_epoch: int,
    operator_file: pathlib.Path,
    occurred_at_ms: int,
) -> Mapping[str, Any]:
    if action not in {"pause", "resume"}:
        raise CutoverError("replay control action is invalid")
    scope = _scope(config, tenant, domain)
    safe_ms(occurred_at_ms, "replay control")
    if isinstance(expected_epoch, bool) or not isinstance(expected_epoch, int) or not 0 <= expected_epoch <= etl.MAX_WIRE_INTEGER:
        raise CutoverError("authority epoch precondition is invalid")
    if action == "resume":
        _require_maintenance_window(config, occurred_at_ms)
    operator = _authorized_operator(config, operator_file)
    paused = int(action == "pause")
    connection = open_state(state_path)
    try:
        connection.execute("BEGIN IMMEDIATE")
        row = _state(connection, scope)
        if occurred_at_ms < row["updated_at_ms"]:
            raise CutoverError("replay control time precedes the current authority state")
        if row["epoch"] != expected_epoch:
            _record_signal(connection, scope, "authority_contention", occurred_at_ms)
            connection.commit()
            raise CutoverError("authority epoch precondition failed")
        if row["phase"] not in {"shadow_read", *MIRROR_PHASES}:
            raise CutoverError("replay controls are unavailable in this authority phase")
        if row["replay_paused"] == paused:
            raise CutoverError("replay control is already in the requested state")
        result = _append_audit(
            connection,
            row,
            action=action,
            to_phase=row["phase"],
            operator_digest=operator,
            evidence={"replay_paused": bool(paused)},
            occurred_at_ms=occurred_at_ms,
            paused=paused,
        )
        connection.commit()
        return result
    except BaseException:
        connection.rollback()
        raise
    finally:
        connection.close()


def _normalize_result(rows: Any, rule: Mapping[str, Any]) -> list[dict[str, Any]]:
    if (
        not isinstance(rows, list)
        or len(rows) > MAX_SHADOW_ROWS
        or not all(isinstance(row, dict) and len(row) <= MAX_SHADOW_FIELDS for row in rows)
    ):
        raise CutoverError("shadow result shape is invalid")
    ignored = set(rule["ignored_fields"])
    casefold = set(rule["casefold_fields"])
    timestamps = set(rule["timestamp_fields"])
    precision = rule["timestamp_precision_ms"]
    required_fields = set(rule["order_by"]) | casefold | timestamps
    normalized: list[dict[str, Any]] = []
    for row in rows:
        if not required_fields.issubset(row):
            raise CutoverError("shadow result is missing a configured normalization field")
        output: dict[str, Any] = {}
        for key, value in row.items():
            etl.quote(key)
            if key in ignored:
                continue
            if key in casefold:
                if not isinstance(value, str):
                    raise CutoverError("shadow collation value is invalid")
                value = etl.unicodedata.normalize("NFKC", value).casefold().strip()
            if key in timestamps and value is not None:
                if isinstance(value, bool) or not isinstance(value, int):
                    raise CutoverError("shadow timestamp value is invalid")
                value = value - (value % precision)
            output[key] = value
        normalized.append(output)
    order_by = rule["order_by"]
    normalized.sort(key=lambda row: tuple(etl.canonical(row.get(column)) for column in order_by))
    return normalized


def compare_shadow(
    state_path: pathlib.Path,
    config: CutoverConfig,
    domain: str,
    observation_path: pathlib.Path,
    observed_at_ms: int,
) -> Mapping[str, Any]:
    safe_ms(observed_at_ms, "shadow observation")
    try:
        payload = etl.bounded_regular_bytes(
            observation_path, MAX_CONTROL_FILE_BYTES, "shadow observation"
        )
        raw = etl.strict_json(payload.decode("utf-8"))
    except (etl.EtlError, UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
        raise CutoverError("shadow observation is unreadable or invalid") from error
    if not isinstance(raw, dict) or set(raw) != {"query_class", "tenant", "legacy", "d1"}:
        raise CutoverError("shadow observation envelope is invalid")
    query_class = raw["query_class"]
    if query_class not in config.shadow_queries:
        raise CutoverError("shadow query class is not approved")
    scope = _scope(config, raw["tenant"], domain)
    rule = config.shadow_queries[query_class]
    normalized_legacy = _normalize_result(raw["legacy"], rule)
    normalized_d1 = _normalize_result(raw["d1"], rule)
    legacy_digest = etl.sha256_json(normalized_legacy)
    d1_digest = etl.sha256_json(normalized_d1)
    raw_legacy = etl.sha256_json(raw["legacy"])
    raw_d1 = etl.sha256_json(raw["d1"])
    if legacy_digest == d1_digest:
        if raw_legacy == raw_d1:
            classification = "match"
        else:
            raw_legacy_sorted = sorted(
                raw["legacy"],
                key=lambda row: tuple(etl.canonical(row[column]) for column in rule["order_by"]),
            )
            raw_d1_sorted = sorted(
                raw["d1"],
                key=lambda row: tuple(etl.canonical(row[column]) for column in rule["order_by"]),
            )
            classification = (
                "ordering_only"
                if etl.sha256_json(raw_legacy_sorted) == etl.sha256_json(raw_d1_sorted)
                else "match"
            )
    elif not normalized_legacy or not normalized_d1:
        classification = "missing"
    else:
        classification = "semantic_mismatch"
    record = {
        "tenant_digest": scope[0],
        "domain": domain,
        "query_class": query_class,
        "legacy_digest": legacy_digest,
        "d1_digest": d1_digest,
        "classification": classification,
        "normalizations_applied": raw_legacy != legacy_digest or raw_d1 != d1_digest,
        "observed_at_ms": observed_at_ms,
    }
    observation_digest = etl.sha256_json(record)
    connection = open_state(state_path)
    try:
        connection.execute("BEGIN IMMEDIATE")
        row = _state(connection, scope)
        if row["phase"] not in {
            "shadow_read",
            "dual_write",
            "d1_authoritative",
            "rolled_back",
        }:
            raise CutoverError("shadow comparisons are unavailable in this authority phase")
        if observed_at_ms < row["updated_at_ms"]:
            raise CutoverError("shadow observation predates the current authority state")
        connection.execute(
            """INSERT OR IGNORE INTO shadow_results(
                 observation_digest, tenant_digest, domain, query_class, legacy_digest,
                 d1_digest, classification, normalizations_applied, observed_at_ms
               ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (
                observation_digest,
                scope[0],
                domain,
                query_class,
                legacy_digest,
                d1_digest,
                classification,
                int(record["normalizations_applied"]),
                observed_at_ms,
            ),
        )
        connection.commit()
    except BaseException:
        connection.rollback()
        raise
    finally:
        connection.close()
    return {"observation_digest": observation_digest, **record}


def capture_events(
    state_path: pathlib.Path,
    config: CutoverConfig,
    domain: str,
    events_path: pathlib.Path,
    captured_at_ms: int,
) -> Mapping[str, int]:
    safe_ms(captured_at_ms, "capture")
    table_lookup = {table.name: table for table in config.plan.tables}
    try:
        payload = etl.bounded_regular_bytes(
            events_path, MAX_CAPTURE_FILE_BYTES, "capture event file"
        )
    except etl.EtlError as error:
        raise CutoverError("capture event file is unreadable") from error
    encoded_lines = payload.split(b"\n")
    if encoded_lines and encoded_lines[-1] == b"":
        encoded_lines.pop()
    if (
        not encoded_lines
        or len(encoded_lines) > MAX_CAPTURE_EVENTS
        or any(not line or len(line) > MAX_CAPTURE_EVENT_BYTES for line in encoded_lines)
    ):
        raise CutoverError("capture event file is outside its safe record bounds")
    inserted = 0
    duplicates = 0
    connection = open_state(state_path)
    try:
        connection.execute("BEGIN IMMEDIATE")
        for encoded_line in encoded_lines:
            try:
                line = encoded_line.decode("utf-8")
                event = etl.strict_json(line)
            except (UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
                raise CutoverError("capture event file contains invalid JSON") from error
            required = {
                "event_id",
                "tenant",
                "domain",
                "sequence",
                "authority_epoch",
                "source_authority",
                "occurred_at_ms",
                "operation",
            }
            if not isinstance(event, dict) or set(event) != required:
                raise CutoverError("capture event envelope is invalid")
            if event["domain"] != domain:
                raise CutoverError("capture event domain differs from command scope")
            scope = _scope(config, event["tenant"], domain)
            if not isinstance(event["event_id"], str) or not etl.SAFE_LABEL.fullmatch(event["event_id"]):
                raise CutoverError("capture event identifier is invalid")
            if event["source_authority"] not in {"legacy", "d1"}:
                raise CutoverError("capture event source authority is invalid")
            if (
                isinstance(event["sequence"], bool)
                or not isinstance(event["sequence"], int)
                or not 1 <= event["sequence"] <= etl.MAX_WIRE_INTEGER
            ):
                raise CutoverError("capture event sequence is invalid")
            safe_ms(event["occurred_at_ms"], "capture event occurrence")
            if event["occurred_at_ms"] > captured_at_ms:
                raise CutoverError("capture event occurrence follows its capture time")
            operation = event["operation"]
            if (
                not isinstance(operation, dict)
                or operation.get("table") not in table_lookup
            ):
                raise CutoverError("capture operation table is invalid")
            table = table_lookup[operation["table"]]
            if operation.get("kind") == "upsert":
                if set(operation) != {"kind", "table", "source_row"} or not isinstance(
                    operation.get("source_row"), dict
                ):
                    raise CutoverError("capture operation source shape is invalid")
                allowed_source_fields = {
                    source for column in table.columns for source in column.input_sources()
                }
                if (
                    len(operation["source_row"]) > len(allowed_source_fields)
                    or not set(operation["source_row"]).issubset(allowed_source_fields)
                ):
                    raise CutoverError("capture operation source shape is invalid")
            elif operation.get("kind") == "delete":
                if set(operation) != {"kind", "table", "source_key"} or not isinstance(
                    operation.get("source_key"), dict
                ):
                    raise CutoverError("capture delete key shape is invalid")
                columns_by_target = {column.target: column for column in table.columns}
                if table.tenant_column is None:
                    raise CutoverError("capture delete requires an explicit target tenant column")
                delete_targets = {*table.primary_key, table.tenant_column}
                delete_sources: set[str] = set()
                for target in delete_targets:
                    sources = columns_by_target[target].input_sources()
                    if len(sources) != 1:
                        raise CutoverError("capture delete key cannot derive its source key")
                    delete_sources.add(sources[0])
                if set(operation["source_key"]) != delete_sources:
                    raise CutoverError("capture delete key shape is invalid")
            else:
                raise CutoverError("capture operation kind is invalid")
            digest = etl.sha256_json(event)
            existing = connection.execute(
                """SELECT event_digest FROM captured_events
                   WHERE tenant_digest = ? AND domain = ? AND event_id = ?""",
                (*scope, event["event_id"]),
            ).fetchone()
            if existing:
                if existing["event_digest"] != digest:
                    raise CutoverError("capture event identifier was reused with different data")
                duplicates += 1
                continue
            state = _state(connection, scope)
            if state["phase"] not in MIRROR_PHASES or not state["mirror_enabled"]:
                raise CutoverError("capture requires an authoritative writer with a controlled mirror")
            if event["source_authority"] != state["writer"]:
                raise CutoverError("capture event came from the non-authoritative writer")
            if (
                isinstance(event["authority_epoch"], bool)
                or not isinstance(event["authority_epoch"], int)
                or event["authority_epoch"] != state["epoch"]
            ):
                raise CutoverError("capture event authority epoch is stale")
            previous_sequence = connection.execute(
                """SELECT COALESCE(MAX(sequence), 0) AS value FROM captured_events
                   WHERE tenant_digest = ? AND domain = ?""",
                scope,
            ).fetchone()["value"]
            if event["sequence"] != previous_sequence + 1:
                raise CutoverError("capture event sequence is not contiguous")
            try:
                connection.execute(
                    """INSERT INTO captured_events(
                         event_id, tenant_digest, domain, sequence, authority_epoch,
                         source_authority, event_digest,
                         payload_json, state, occurred_at_ms, captured_at_ms
                       ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)""",
                    (
                        event["event_id"],
                        scope[0],
                        domain,
                        event["sequence"],
                        event["authority_epoch"],
                        event["source_authority"],
                        digest,
                        etl.canonical(event),
                        event["occurred_at_ms"],
                        captured_at_ms,
                    ),
                )
            except sqlite3.IntegrityError as error:
                raise CutoverError("capture sequence conflicts with an existing event") from error
            inserted += 1
        connection.commit()
    except BaseException:
        connection.rollback()
        raise
    finally:
        connection.close()
    return {"captured": inserted, "idempotent_duplicates": duplicates}


def _ensure_replay_ledger(connection: sqlite3.Connection) -> None:
    existing = connection.execute(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_frame_change_ledger'"
    ).fetchone()
    if existing is not None:
        primary_key = [
            row["name"]
            for row in sorted(
                connection.execute("PRAGMA table_info(_frame_change_ledger)"),
                key=lambda row: row["pk"],
            )
            if row["pk"]
        ]
        if primary_key != ["tenant_digest", "domain", "event_id"]:
            raise CutoverError("target replay ledger schema is incompatible")
    connection.execute(
        """CREATE TABLE IF NOT EXISTS _frame_change_ledger (
             event_id TEXT NOT NULL,
             event_digest TEXT NOT NULL CHECK(length(event_digest) = 64),
             tenant_digest TEXT NOT NULL CHECK(length(tenant_digest) = 64),
             domain TEXT NOT NULL,
             sequence INTEGER NOT NULL,
             applied_at_ms INTEGER NOT NULL,
             PRIMARY KEY(tenant_digest, domain, event_id),
             UNIQUE(tenant_digest, domain, sequence)
           )"""
    )
    connection.commit()


def _apply_event(
    target: sqlite3.Connection,
    config: CutoverConfig,
    event: Mapping[str, Any],
    tenant_hash: str,
    event_digest: str,
    applied_at_ms: int,
) -> None:
    table_lookup = {table.name: table for table in config.plan.tables}
    operation = event["operation"]
    table = table_lookup[operation["table"]]
    if operation["kind"] == "upsert":
        if set(operation) != {"kind", "table", "source_row"} or not isinstance(
            operation["source_row"], dict
        ):
            raise ValueError("invalid_source_row")
        row = etl.transform_row(table, event["tenant"], operation["source_row"])
        columns = [column.target for column in table.columns]
        column_sql = ", ".join(etl.quote(column) for column in columns)
        placeholders = ", ".join("?" for _ in columns)
        update_columns = [column for column in columns if column not in table.primary_key]
        updates = ", ".join(
            f"{etl.quote(column)} = excluded.{etl.quote(column)}"
            for column in update_columns
        )
        conflict = ", ".join(etl.quote(column) for column in table.primary_key)
        conflict_action = f"DO UPDATE SET {updates}" if updates else "DO NOTHING"
        sql = (
            f"INSERT INTO {etl.quote(table.target_name)} ({column_sql}) VALUES ({placeholders}) "
            f"ON CONFLICT ({conflict}) {conflict_action}"
        )
        target.execute(sql, tuple(row[column] for column in columns))
    elif operation["kind"] == "delete":
        if set(operation) != {"kind", "table", "source_key"} or not isinstance(
            operation["source_key"], dict
        ):
            raise ValueError("invalid_source_key")
        columns_by_target = {column.target: column for column in table.columns}
        if table.tenant_column is None:
            raise ValueError("invalid_source_key")
        predicate_targets = list(table.primary_key)
        if table.tenant_column not in predicate_targets:
            predicate_targets.append(table.tenant_column)
        target_sources = {
            target_column: columns_by_target[target_column].input_sources()
            for target_column in predicate_targets
        }
        if any(len(sources) != 1 for sources in target_sources.values()):
            raise ValueError("invalid_source_key")
        expected_sources = {sources[0] for sources in target_sources.values()}
        if set(operation["source_key"]) != expected_sources:
            raise ValueError("invalid_source_key")
        predicate_values = [
            etl.transform_value(
                columns_by_target[target_column],
                operation["source_key"][target_sources[target_column][0]],
            )
            for target_column in predicate_targets
        ]
        tenant_index = predicate_targets.index(table.tenant_column)
        if predicate_values[tenant_index] != event["tenant"]:
            raise ValueError("tenant_mismatch")
        predicate = " AND ".join(
            f"{etl.quote(target_column)} = ?" for target_column in predicate_targets
        )
        target.execute(
            f"DELETE FROM {etl.quote(table.target_name)} WHERE {predicate}",
            tuple(predicate_values),
        )
    else:
        raise ValueError("unsupported_operation")
    target.execute(
        """INSERT INTO _frame_change_ledger(
             event_id, event_digest, tenant_digest, domain, sequence, applied_at_ms
           ) VALUES (?, ?, ?, ?, ?, ?)""",
        (event["event_id"], event_digest, tenant_hash, event["domain"], event["sequence"], applied_at_ms),
    )


def replay_events(
    state_path: pathlib.Path,
    target_path: pathlib.Path,
    config: CutoverConfig,
    tenant: str,
    domain: str,
    applied_at_ms: int,
    max_events: int,
    inject_after_target_commit: bool = False,
) -> Mapping[str, int]:
    if (
        isinstance(max_events, bool)
        or not isinstance(max_events, int)
        or not 1 <= max_events <= MAX_CAPTURE_EVENTS
    ):
        raise CutoverError("replay max_events is outside its safe bound")
    safe_ms(applied_at_ms, "replay")
    scope = _scope(config, tenant, domain)
    state_db = open_state(state_path)
    target = sqlite3.connect(target_path)
    target.row_factory = sqlite3.Row
    target.execute("PRAGMA foreign_keys = ON")
    _ensure_replay_ledger(target)
    applied = 0
    recovered = 0
    dead_lettered = 0
    try:
        state = _state(state_db, scope)
        if state["phase"] not in MIRROR_PHASES or not state["mirror_enabled"]:
            raise CutoverError("replay requires the controlled mirror phase")
        if state["replay_paused"]:
            raise CutoverError("replay is paused by an audited operator control")
        if applied_at_ms < state["updated_at_ms"]:
            raise CutoverError("replay time precedes the current authority state")
        events = state_db.execute(
            """SELECT event_id, sequence FROM captured_events
               WHERE tenant_digest = ? AND domain = ? AND state = 'pending'
               ORDER BY sequence LIMIT ?""",
            (*scope, max_events),
        ).fetchall()
        applied_sequence_row = state_db.execute(
            """SELECT COALESCE(MAX(sequence), 0) AS value FROM captured_events
               WHERE tenant_digest = ? AND domain = ? AND state = 'applied'""",
            scope,
        ).fetchone()
        next_sequence = applied_sequence_row["value"] + 1
        for candidate in events:
            if candidate["sequence"] != next_sequence:
                break
            try:
                state_db.execute("BEGIN IMMEDIATE")
                current = _state(state_db, scope)
                if (
                    current["phase"] not in MIRROR_PHASES
                    or not current["mirror_enabled"]
                    or current["replay_paused"]
                ):
                    raise CutoverError("replay authority changed before the next event boundary")
                row = state_db.execute(
                    """SELECT * FROM captured_events
                       WHERE event_id = ? AND tenant_digest = ? AND domain = ? AND state = 'pending'""",
                    (candidate["event_id"], *scope),
                ).fetchone()
                if row is None:
                    state_db.rollback()
                    applied_sequence_row = state_db.execute(
                        """SELECT COALESCE(MAX(sequence), 0) AS value FROM captured_events
                           WHERE tenant_digest = ? AND domain = ? AND state = 'applied'""",
                        scope,
                    ).fetchone()
                    next_sequence = applied_sequence_row["value"] + 1
                    continue
                if row["sequence"] != next_sequence:
                    state_db.rollback()
                    break
                try:
                    event = etl.strict_json(row["payload_json"])
                except (TypeError, ValueError, json.JSONDecodeError) as error:
                    raise CutoverError("captured event payload is corrupt") from error
                if (
                    not isinstance(event, dict)
                    or etl.sha256_json(event) != row["event_digest"]
                    or event.get("event_id") != row["event_id"]
                    or event.get("domain") != domain
                    or event.get("sequence") != row["sequence"]
                    or event.get("authority_epoch") != row["authority_epoch"]
                    or event.get("source_authority") != row["source_authority"]
                    or not isinstance(event.get("tenant"), str)
                    or etl.tenant_digest(event["tenant"]) != scope[0]
                ):
                    raise CutoverError("captured event payload does not match its durable envelope")
                ledger = target.execute(
                    """SELECT event_digest, tenant_digest, domain, sequence
                       FROM _frame_change_ledger
                       WHERE tenant_digest = ? AND domain = ? AND event_id = ?""",
                    (*scope, row["event_id"]),
                ).fetchone()
                if ledger:
                    if (
                        ledger["event_digest"] != row["event_digest"]
                        or ledger["tenant_digest"] != scope[0]
                        or ledger["domain"] != domain
                        or ledger["sequence"] != row["sequence"]
                    ):
                        raise CutoverError("target replay ledger conflicts with the captured event")
                    state_db.execute(
                        """UPDATE captured_events SET state = 'applied', applied_at_ms = ?
                           WHERE event_id = ? AND state = 'pending'""",
                        (applied_at_ms, row["event_id"]),
                    )
                    state_db.commit()
                    recovered += 1
                    next_sequence += 1
                    continue
                try:
                    target.execute("BEGIN IMMEDIATE")
                    _apply_event(
                        target, config, event, scope[0], row["event_digest"], applied_at_ms
                    )
                    target.commit()
                except (KeyError, TypeError, ValueError, json.JSONDecodeError, sqlite3.IntegrityError) as error:
                    target.rollback()
                    reason = str(error).split(":", maxsplit=1)[0]
                    if not etl.SAFE_LABEL.fullmatch(reason):
                        reason = "invalid_event"
                    state_db.execute(
                        """UPDATE captured_events SET state = 'dead_letter', reason_code = ?
                           WHERE event_id = ? AND state = 'pending'""",
                        (reason, row["event_id"]),
                    )
                    _record_signal(state_db, scope, "replay_write_failure", applied_at_ms)
                    state_db.commit()
                    dead_lettered += 1
                    break
                except sqlite3.Error as error:
                    target.rollback()
                    _record_signal(state_db, scope, "replay_write_failure", applied_at_ms)
                    state_db.commit()
                    raise CutoverError(
                        "target replay write failed; event remains pending"
                    ) from error
                if inject_after_target_commit:
                    _record_signal(state_db, scope, "replay_lost_ack", applied_at_ms)
                    state_db.commit()
                    raise InjectedReplayInterruption
                state_db.execute(
                    """UPDATE captured_events SET state = 'applied', applied_at_ms = ?
                       WHERE event_id = ? AND state = 'pending'""",
                    (applied_at_ms, row["event_id"]),
                )
                state_db.commit()
                applied += 1
                next_sequence += 1
            except BaseException:
                if state_db.in_transaction:
                    state_db.rollback()
                if target.in_transaction:
                    target.rollback()
                raise
    finally:
        if target.in_transaction:
            target.rollback()
        if state_db.in_transaction:
            state_db.rollback()
        target.close()
        state_db.close()
    return {"applied": applied, "recovered_after_commit": recovered, "dead_lettered": dead_lettered}


def _verify_audit_connection(
    connection: sqlite3.Connection,
    scope: tuple[str, str],
) -> Mapping[str, Any]:
    state = _state(connection, scope)
    rows = connection.execute(
        """SELECT * FROM authority_audit
           WHERE tenant_digest = ? AND domain = ? ORDER BY to_epoch""",
        scope,
    ).fetchall()
    previous_hash = "0" * 64
    expected_epoch = 0
    expected_phase = "legacy_authoritative"
    expected_paused = False
    last_time: int | None = None
    for row in rows:
        record = {
            "previous_hash": row["previous_hash"],
            "tenant_digest": row["tenant_digest"],
            "domain": row["domain"],
            "action": row["action"],
            "from_phase": row["from_phase"],
            "to_phase": row["to_phase"],
            "from_epoch": row["from_epoch"],
            "to_epoch": row["to_epoch"],
            "operator_digest": row["operator_digest"],
            "evidence_digest": row["evidence_digest"],
            "occurred_at_ms": row["occurred_at_ms"],
        }
        transition_valid = (
            row["action"] == "transition"
            and row["to_phase"] in ALLOWED_TRANSITIONS.get(expected_phase, set())
        )
        control_valid = (
            (
                (row["action"] == "pause" and not expected_paused)
                or (row["action"] == "resume" and expected_paused)
            )
            and row["to_phase"] == expected_phase
            and expected_phase in {"shadow_read", *MIRROR_PHASES}
        )
        if (
            row["previous_hash"] != previous_hash
            or row["from_epoch"] != expected_epoch
            or row["to_epoch"] != expected_epoch + 1
            or row["from_phase"] != expected_phase
            or not (transition_valid or control_valid)
            or not etl.SHA256.fullmatch(row["operator_digest"])
            or not etl.SHA256.fullmatch(row["evidence_digest"])
            or (last_time is not None and row["occurred_at_ms"] < last_time)
            or etl.sha256_json(record) != row["audit_hash"]
        ):
            raise CutoverError("authority audit hash chain is invalid")
        previous_hash = row["audit_hash"]
        expected_epoch = row["to_epoch"]
        expected_phase = row["to_phase"]
        expected_paused = row["action"] == "pause" if row["action"] != "transition" else False
        last_time = row["occurred_at_ms"]
    if (
        state["epoch"] != expected_epoch
        or state["phase"] != expected_phase
        or bool(state["replay_paused"]) != expected_paused
        or state["audit_head"] != previous_hash
        or (last_time is not None and state["updated_at_ms"] != last_time)
    ):
        raise CutoverError("authority audit head does not bind the current state")
    return {
        "tenant_digest": scope[0],
        "domain": scope[1],
        "audit_entries": len(rows),
        "epoch": expected_epoch,
        "audit_head": previous_hash,
        "valid": True,
    }


def verify_audit_chain(
    state_path: pathlib.Path,
    config: CutoverConfig,
    tenant: str,
    domain: str,
) -> Mapping[str, Any]:
    scope = _scope(config, tenant, domain)
    connection = open_state(state_path)
    try:
        return _verify_audit_connection(connection, scope)
    finally:
        connection.close()


def verify_writer_fence(
    state_path: pathlib.Path,
    config: CutoverConfig,
    tenant: str,
    domain: str,
    writer: str,
    expected_epoch: int,
) -> Mapping[str, Any]:
    if writer not in {"legacy", "d1"}:
        raise CutoverError("writer fence authority is invalid")
    if (
        isinstance(expected_epoch, bool)
        or not isinstance(expected_epoch, int)
        or not 0 <= expected_epoch <= etl.MAX_WIRE_INTEGER
    ):
        raise CutoverError("writer fence epoch is invalid")
    scope = _scope(config, tenant, domain)
    connection = open_state(state_path)
    try:
        connection.execute("BEGIN")
        row = _state(connection, scope)
        audit = _verify_audit_connection(connection, scope)
        if row["writer"] != writer or row["epoch"] != expected_epoch:
            raise CutoverError("writer fence is stale or belongs to the other authority")
        record = {
            "tenant_digest": scope[0],
            "domain": domain,
            "writer": writer,
            "epoch": expected_epoch,
            "audit_head": audit["audit_head"],
        }
        return {**record, "fence_digest": etl.sha256_json(record), "authorized": True}
    finally:
        connection.close()


def status(
    state_path: pathlib.Path,
    config: CutoverConfig,
    tenant: str,
    domain: str,
    now_ms: int,
) -> Mapping[str, Any]:
    safe_ms(now_ms, "status")
    scope = _scope(config, tenant, domain)
    connection = open_state(state_path)
    try:
        state = _state(connection, scope)
        if now_ms < state["updated_at_ms"]:
            raise CutoverError("status time precedes the current authority state")
        counts = {
            row["state"]: row["count"]
            for row in connection.execute(
                """SELECT state, COUNT(*) AS count FROM captured_events
                   WHERE tenant_digest = ? AND domain = ? GROUP BY state""",
                scope,
            )
        }
        oldest = connection.execute(
            """SELECT MIN(occurred_at_ms) AS value FROM captured_events
               WHERE tenant_digest = ? AND domain = ? AND state = 'pending'""",
            scope,
        ).fetchone()["value"]
        shadow = {
            row["classification"]: row["count"]
            for row in connection.execute(
                """SELECT classification, COUNT(*) AS count FROM shadow_results
                   WHERE tenant_digest = ? AND domain = ? GROUP BY classification""",
                scope,
            )
        }
        shadow_health = _shadow_health(connection, config, scope, now_ms)
        operational_health = _operational_health(connection, config, scope, now_ms)
        recent_contention = operational_health["authority_contention"]
        recent_write_failures = operational_health["replay_write_failure"]
        recent_lost_ack = operational_health["replay_lost_ack"]
        pending_lag = 0 if oldest is None else max(0, now_ms - oldest)
        pending = counts.get("pending", 0)
        dead_letters = counts.get("dead_letter", 0)
        rollback_ready = bool(
            state["phase"] == "d1_authoritative"
            and state["rollback_ready"]
            and pending == 0
            and dead_letters == 0
        )
        audit = _verify_audit_connection(connection, scope)
        alerts = {
            "authority_contention": {
                "active": recent_contention > config.max_contention_events,
                "current": recent_contention,
                "limit": config.max_contention_events,
            },
            "dead_letter": {
                "active": dead_letters > config.max_dead_letter_events,
                "current": dead_letters,
                "limit": config.max_dead_letter_events,
            },
            "replay_lag": {
                "active": pending_lag > config.max_pending_lag_ms,
                "current_ms": pending_lag,
                "limit_ms": config.max_pending_lag_ms,
            },
            "replay_write_failure": {
                "active": recent_write_failures > 0,
                "current": recent_write_failures,
                "limit": 0,
            },
            "replay_lost_ack": {
                "active": recent_lost_ack > 0,
                "current": recent_lost_ack,
                "limit": 0,
            },
            "shadow_coverage": {
                "active": state["phase"] in {"dual_write", "d1_authoritative", "finalized"}
                and not shadow_health["coverage_complete"],
                "current": int(shadow_health["coverage_complete"]),
                "required": 1,
            },
            "shadow_mismatch": {
                "active": shadow_health["mismatches"] > config.max_shadow_mismatches,
                "current": shadow_health["mismatches"],
                "limit": config.max_shadow_mismatches,
            },
            "rollback_readiness": {
                "active": state["phase"] in {"dual_write", "d1_authoritative"}
                and not rollback_ready,
                "current": int(rollback_ready),
                "required": 1,
            },
        }
        return {
            "tenant_digest": scope[0],
            "domain": domain,
            "phase": state["phase"],
            "writer": state["writer"],
            "mirror_enabled": bool(state["mirror_enabled"]),
            "replay_paused": bool(state["replay_paused"]),
            "epoch": state["epoch"],
            "pending_events": pending,
            "dead_letter_events": dead_letters,
            "applied_events": counts.get("applied", 0),
            "oldest_pending_lag_ms": pending_lag,
            "shadow_classifications": shadow,
            "latest_shadow_window": shadow_health,
            "latest_operational_window": operational_health,
            "recent_lost_acknowledgements": recent_lost_ack,
            "rollback_ready": rollback_ready,
            "slo": {
                "max_pending_lag_ms": config.max_pending_lag_ms,
                "max_shadow_mismatches": config.max_shadow_mismatches,
                "max_dead_letter_events": config.max_dead_letter_events,
                "max_contention_events": config.max_contention_events,
                "shadow_window_ms": config.shadow_window_ms,
                "minimum_shadow_observations": config.minimum_shadow_observations,
            },
            "alerts": alerts,
            "audit_chain_valid": audit["valid"],
            "audit_head": state["audit_head"],
        }
    finally:
        connection.close()


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    root.add_argument("--config", type=pathlib.Path, required=True)
    root.add_argument("--state", type=pathlib.Path, required=True)
    commands = root.add_subparsers(dest="command", required=True)

    initialize = commands.add_parser("init")
    initialize.add_argument("--tenant", required=True)
    initialize.add_argument("--domain", required=True)
    initialize.add_argument("--at-ms", type=int, required=True)

    transition_command = commands.add_parser("transition")
    transition_command.add_argument("--tenant", required=True)
    transition_command.add_argument("--domain", required=True)
    transition_command.add_argument("--to", required=True)
    transition_command.add_argument("--expected-epoch", type=int, required=True)
    transition_command.add_argument("--operator-file", type=pathlib.Path, required=True)
    transition_command.add_argument("--evidence", type=pathlib.Path, required=True)
    transition_command.add_argument("--at-ms", type=int, required=True)

    control = commands.add_parser("control")
    control.add_argument("--tenant", required=True)
    control.add_argument("--domain", required=True)
    control.add_argument("--action", choices=("pause", "resume"), required=True)
    control.add_argument("--expected-epoch", type=int, required=True)
    control.add_argument("--operator-file", type=pathlib.Path, required=True)
    control.add_argument("--at-ms", type=int, required=True)

    shadow = commands.add_parser("shadow")
    shadow.add_argument("--domain", required=True)
    shadow.add_argument("--observation", type=pathlib.Path, required=True)
    shadow.add_argument("--at-ms", type=int, required=True)
    shadow.add_argument("--report", type=pathlib.Path, required=True)

    capture = commands.add_parser("capture")
    capture.add_argument("--domain", required=True)
    capture.add_argument("--events", type=pathlib.Path, required=True)
    capture.add_argument("--at-ms", type=int, required=True)

    replay = commands.add_parser("replay")
    replay.add_argument("--tenant", required=True)
    replay.add_argument("--domain", required=True)
    replay.add_argument("--target", type=pathlib.Path, required=True)
    replay.add_argument("--at-ms", type=int, required=True)
    replay.add_argument("--max-events", type=int, default=1000)
    replay.add_argument("--inject-after-target-commit", action="store_true", help=argparse.SUPPRESS)

    status_command = commands.add_parser("status")
    status_command.add_argument("--tenant", required=True)
    status_command.add_argument("--domain", required=True)
    status_command.add_argument("--now-ms", type=int, required=True)
    status_command.add_argument("--report", type=pathlib.Path, required=True)

    fence = commands.add_parser("verify-fence")
    fence.add_argument("--tenant", required=True)
    fence.add_argument("--domain", required=True)
    fence.add_argument("--writer", choices=("legacy", "d1"), required=True)
    fence.add_argument("--expected-epoch", type=int, required=True)
    fence.add_argument("--report", type=pathlib.Path, required=True)

    audit = commands.add_parser("verify-audit")
    audit.add_argument("--tenant", required=True)
    audit.add_argument("--domain", required=True)
    audit.add_argument("--report", type=pathlib.Path, required=True)
    return root


def main(arguments: Sequence[str] | None = None) -> int:
    args = parser().parse_args(arguments)
    try:
        config = load_config(args.config)
        if args.command == "init":
            result = initialize_scope(args.state, config, args.tenant, args.domain, args.at_ms)
        elif args.command == "transition":
            result = transition(
                args.state,
                config,
                args.tenant,
                args.domain,
                args.to,
                args.expected_epoch,
                args.operator_file,
                args.evidence,
                args.at_ms,
            )
        elif args.command == "control":
            result = replay_control(
                args.state,
                config,
                args.tenant,
                args.domain,
                args.action,
                args.expected_epoch,
                args.operator_file,
                args.at_ms,
            )
        elif args.command == "shadow":
            result = compare_shadow(
                args.state, config, args.domain, args.observation, args.at_ms
            )
            etl.atomic_private_write(args.report, f"{etl.canonical(result)}\n".encode("utf-8"))
        elif args.command == "capture":
            result = capture_events(args.state, config, args.domain, args.events, args.at_ms)
        elif args.command == "replay":
            result = replay_events(
                args.state,
                args.target,
                config,
                args.tenant,
                args.domain,
                args.at_ms,
                args.max_events,
                args.inject_after_target_commit,
            )
        elif args.command == "status":
            result = status(args.state, config, args.tenant, args.domain, args.now_ms)
            etl.atomic_private_write(args.report, f"{etl.canonical(result)}\n".encode("utf-8"))
        elif args.command == "verify-fence":
            result = verify_writer_fence(
                args.state,
                config,
                args.tenant,
                args.domain,
                args.writer,
                args.expected_epoch,
            )
            etl.atomic_private_write(args.report, f"{etl.canonical(result)}\n".encode("utf-8"))
        else:
            result = verify_audit_chain(
                args.state, config, args.tenant, args.domain
            )
            etl.atomic_private_write(args.report, f"{etl.canonical(result)}\n".encode("utf-8"))
        print(etl.canonical(result))
        return 0
    except InjectedReplayInterruption:
        print("replay interrupted after durable target commit; rerun to recover from the target ledger", file=sys.stderr)
        return 75
    except (CutoverError, etl.EtlError, OSError, sqlite3.Error) as error:
        print(f"cutover control failed safely: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
