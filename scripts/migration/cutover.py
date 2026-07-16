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
import sqlite3
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


class CutoverError(Exception):
    """An operator-safe failure which must never contain payload values."""


class InjectedReplayInterruption(Exception):
    """A fault after target commit but before source event acknowledgement."""


@dataclass(frozen=True)
class CutoverConfig:
    plan: etl.Plan
    operator_digests: frozenset[str]
    domains: frozenset[str]
    shadow_queries: Mapping[str, Mapping[str, Any]]


def safe_ms(value: Any, description: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= etl.MAX_WIRE_INTEGER:
        raise CutoverError(f"{description} timestamp is invalid")
    return value


def load_config(path: pathlib.Path) -> CutoverConfig:
    plan = etl.load_plan(path)
    raw = plan.raw.get("cutover")
    if not isinstance(raw, dict) or set(raw) != {
        "authorized_operator_digests",
        "domains",
        "shadow_queries",
    }:
        raise CutoverError("cutover configuration is invalid")
    operators = raw["authorized_operator_digests"]
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
        or not all(isinstance(item, str) and etl.SAFE_LABEL.fullmatch(item) for item in domains)
    ):
        raise CutoverError("cutover domains are invalid")
    if not isinstance(queries, dict) or not queries:
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
    return CutoverConfig(plan, frozenset(operators), frozenset(domains), queries)


def open_state(path: pathlib.Path) -> sqlite3.Connection:
    etl.private_directory(path.parent)
    if path.is_symlink():
        raise CutoverError("cutover state path cannot be a symbolic link")
    if not path.exists():
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
        if hasattr(os, "O_NOFOLLOW"):
            flags |= os.O_NOFOLLOW
        descriptor = os.open(path, flags, 0o600)
        os.close(descriptor)
    elif not path.is_file():
        raise CutoverError("cutover state path is not a regular file")
    path.chmod(0o600)
    connection = sqlite3.connect(path)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA foreign_keys = ON")
    connection.execute("PRAGMA journal_mode = WAL")
    connection.executescript(
        """
        CREATE TABLE IF NOT EXISTS authority_state (
          tenant_digest TEXT NOT NULL CHECK(length(tenant_digest) = 64),
          domain TEXT NOT NULL,
          phase TEXT NOT NULL,
          writer TEXT NOT NULL CHECK(writer IN ('legacy', 'd1')),
          mirror_enabled INTEGER NOT NULL CHECK(mirror_enabled IN (0, 1)),
          replay_paused INTEGER NOT NULL CHECK(replay_paused IN (0, 1)),
          epoch INTEGER NOT NULL,
          audit_head TEXT NOT NULL CHECK(length(audit_head) = 64),
          updated_at_ms INTEGER NOT NULL,
          PRIMARY KEY(tenant_digest, domain)
        );
        CREATE TABLE IF NOT EXISTS authority_audit (
          audit_hash TEXT PRIMARY KEY NOT NULL CHECK(length(audit_hash) = 64),
          previous_hash TEXT NOT NULL CHECK(length(previous_hash) = 64),
          tenant_digest TEXT NOT NULL,
          domain TEXT NOT NULL,
          action TEXT NOT NULL,
          from_phase TEXT NOT NULL,
          to_phase TEXT NOT NULL,
          from_epoch INTEGER NOT NULL,
          to_epoch INTEGER NOT NULL CHECK(to_epoch = from_epoch + 1),
          operator_digest TEXT NOT NULL CHECK(length(operator_digest) = 64),
          evidence_digest TEXT NOT NULL CHECK(length(evidence_digest) = 64),
          occurred_at_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS captured_events (
          event_id TEXT PRIMARY KEY NOT NULL,
          tenant_digest TEXT NOT NULL,
          domain TEXT NOT NULL,
          sequence INTEGER NOT NULL CHECK(sequence > 0),
          authority_epoch INTEGER NOT NULL,
          event_digest TEXT NOT NULL CHECK(length(event_digest) = 64),
          payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
          state TEXT NOT NULL CHECK(state IN ('pending', 'applied', 'dead_letter')),
          reason_code TEXT,
          occurred_at_ms INTEGER NOT NULL,
          captured_at_ms INTEGER NOT NULL,
          applied_at_ms INTEGER,
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
          classification TEXT NOT NULL,
          normalizations_applied INTEGER NOT NULL CHECK(normalizations_applied IN (0, 1)),
          observed_at_ms INTEGER NOT NULL
        );
        CREATE TRIGGER IF NOT EXISTS authority_audit_immutable_update
        BEFORE UPDATE ON authority_audit BEGIN SELECT RAISE(ABORT, 'authority audit is immutable'); END;
        CREATE TRIGGER IF NOT EXISTS authority_audit_immutable_delete
        BEFORE DELETE ON authority_audit BEGIN SELECT RAISE(ABORT, 'authority audit is immutable'); END;
        """
    )
    connection.commit()
    return connection


def _scope(config: CutoverConfig, tenant: str, domain: str) -> tuple[str, str]:
    if not isinstance(tenant, str) or not etl.SAFE_LABEL.fullmatch(tenant):
        raise CutoverError("tenant scope is invalid")
    if domain not in config.domains:
        raise CutoverError("cutover domain is not authorized by the plan")
    return etl.tenant_digest(tenant), domain


def _state(connection: sqlite3.Connection, scope: tuple[str, str]) -> sqlite3.Row:
    row = connection.execute(
        "SELECT * FROM authority_state WHERE tenant_digest = ? AND domain = ?", scope
    ).fetchone()
    if row is None:
        raise CutoverError("authority scope has not been initialized")
    if PHASE_WRITER.get(row["phase"]) != row["writer"]:
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
                 epoch, audit_head, updated_at_ms
               ) VALUES (?, ?, 'legacy_authoritative', 'legacy', 0, 0, 0, ?, ?)""",
            (*scope, "0" * 64, occurred_at_ms),
        )
        connection.commit()
    except BaseException:
        connection.rollback()
        raise
    finally:
        connection.close()
    return {"tenant_digest": scope[0], "domain": domain, "phase": "legacy_authoritative", "writer": "legacy", "epoch": 0}


def _authorized_operator(config: CutoverConfig, operator_file: pathlib.Path) -> str:
    try:
        value = operator_file.read_bytes()
    except OSError as error:
        raise CutoverError("operator credential file is unreadable") from error
    if not value or len(value) > 1024:
        raise CutoverError("operator credential is invalid")
    digest = hashlib.sha256(value.rstrip(b"\r\n")).hexdigest()
    if not any(hmac.compare_digest(digest, allowed) for allowed in config.operator_digests):
        raise CutoverError("operator is not authorized for migration controls")
    return digest


def _load_evidence(path: pathlib.Path) -> Mapping[str, Any]:
    raw = etl.load_json(path, "transition evidence")
    if not isinstance(raw, dict) or set(raw) - SAFE_EVIDENCE_KEYS:
        raise CutoverError("transition evidence contains unsupported fields")
    for key, value in raw.items():
        if key == "reconciliation_sha256":
            if not isinstance(value, str) or not etl.SHA256.fullmatch(value):
                raise CutoverError("transition reconciliation digest is invalid")
        elif key in COUNT_EVIDENCE_KEYS:
            if isinstance(value, bool) or not isinstance(value, int) or value < 0:
                raise CutoverError("transition count evidence is invalid")
        elif not isinstance(value, bool):
            raise CutoverError("transition evidence must contain only booleans, counts, and digests")
    return raw


def _gate_transition(current: str, target: str, evidence: Mapping[str, Any]) -> None:
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
        required = {"d1_fenced": True, "legacy_caught_up": True, "rollback_rehearsed": True}
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
    if target in {"d1_authoritative", "finalized"}:
        reconciliation_digest = evidence.get("reconciliation_sha256")
        if not isinstance(reconciliation_digest, str) or not etl.SHA256.fullmatch(reconciliation_digest):
            raise CutoverError("transition requires a reconciliation evidence digest")
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
             audit_head = ?, updated_at_ms = ?
           WHERE tenant_digest = ? AND domain = ? AND epoch = ?""",
        (
            to_phase,
            PHASE_WRITER[to_phase],
            int(to_phase in {"dual_write", "rolled_back"}),
            paused,
            next_epoch,
            audit_hash,
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
    operator = _authorized_operator(config, operator_file)
    evidence = _load_evidence(evidence_path)
    connection = open_state(state_path)
    try:
        connection.execute("BEGIN IMMEDIATE")
        row = _state(connection, scope)
        if row["epoch"] != expected_epoch:
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
        live_shadow_mismatches = connection.execute(
            """SELECT COUNT(*) AS count FROM shadow_results
               WHERE tenant_digest = ? AND domain = ?
                 AND classification IN ('semantic_mismatch', 'missing', 'error')""",
            scope,
        ).fetchone()["count"]
        live_values = {
            "pending_events": live_event_counts.get("pending", 0),
            "dead_letter_events": live_event_counts.get("dead_letter", 0),
            "shadow_mismatches": live_shadow_mismatches,
        }
        for key, value in live_values.items():
            if key in evidence and evidence[key] != value:
                raise CutoverError(f"transition evidence disagrees with durable {key}")
        _gate_transition(row["phase"], target_phase, evidence)
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
    operator = _authorized_operator(config, operator_file)
    paused = int(action == "pause")
    connection = open_state(state_path)
    try:
        connection.execute("BEGIN IMMEDIATE")
        row = _state(connection, scope)
        if row["epoch"] != expected_epoch:
            raise CutoverError("authority epoch precondition failed")
        if row["phase"] not in {"shadow_read", "dual_write", "rolled_back"}:
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
    if not isinstance(rows, list) or not all(isinstance(row, dict) for row in rows):
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
    raw = etl.load_json(observation_path, "shadow observation")
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
        row = _state(connection, scope)
        if row["phase"] not in {"shadow_read", "dual_write", "d1_authoritative"}:
            raise CutoverError("shadow comparisons are unavailable in this authority phase")
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
    table_names = {table.name for table in config.plan.tables}
    try:
        lines = events_path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError) as error:
        raise CutoverError("capture event file is unreadable") from error
    inserted = 0
    duplicates = 0
    connection = open_state(state_path)
    try:
        connection.execute("BEGIN IMMEDIATE")
        for line in lines:
            try:
                event = etl.strict_json(line)
            except (ValueError, json.JSONDecodeError) as error:
                raise CutoverError("capture event file contains invalid JSON") from error
            required = {"event_id", "tenant", "domain", "sequence", "authority_epoch", "occurred_at_ms", "operation"}
            if not isinstance(event, dict) or set(event) != required:
                raise CutoverError("capture event envelope is invalid")
            if event["domain"] != domain:
                raise CutoverError("capture event domain differs from command scope")
            scope = _scope(config, event["tenant"], domain)
            state = _state(connection, scope)
            if state["phase"] != "dual_write" or state["writer"] != "legacy":
                raise CutoverError("capture requires legacy authority in the controlled capture phase")
            if event["authority_epoch"] != state["epoch"]:
                raise CutoverError("capture event authority epoch is stale")
            if not isinstance(event["event_id"], str) or not etl.SAFE_LABEL.fullmatch(event["event_id"]):
                raise CutoverError("capture event identifier is invalid")
            if isinstance(event["sequence"], bool) or not isinstance(event["sequence"], int) or event["sequence"] <= 0:
                raise CutoverError("capture event sequence is invalid")
            safe_ms(event["occurred_at_ms"], "capture event occurrence")
            operation = event["operation"]
            if not isinstance(operation, dict) or operation.get("table") not in table_names:
                raise CutoverError("capture operation table is invalid")
            digest = etl.sha256_json(event)
            existing = connection.execute(
                "SELECT event_digest FROM captured_events WHERE event_id = ?", (event["event_id"],)
            ).fetchone()
            if existing:
                if existing["event_digest"] != digest:
                    raise CutoverError("capture event identifier was reused with different data")
                duplicates += 1
                continue
            try:
                connection.execute(
                    """INSERT INTO captured_events(
                         event_id, tenant_digest, domain, sequence, authority_epoch, event_digest,
                         payload_json, state, occurred_at_ms, captured_at_ms
                       ) VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?)""",
                    (
                        event["event_id"],
                        scope[0],
                        domain,
                        event["sequence"],
                        event["authority_epoch"],
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
    connection.execute(
        """CREATE TABLE IF NOT EXISTS _frame_change_ledger (
             event_id TEXT PRIMARY KEY NOT NULL,
             event_digest TEXT NOT NULL CHECK(length(event_digest) = 64),
             tenant_digest TEXT NOT NULL CHECK(length(tenant_digest) = 64),
             domain TEXT NOT NULL,
             sequence INTEGER NOT NULL,
             applied_at_ms INTEGER NOT NULL,
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
    if set(operation) != {"kind", "table", "source_row"} or operation["kind"] != "upsert":
        raise ValueError("unsupported_operation")
    table = table_lookup[operation["table"]]
    if not isinstance(operation["source_row"], dict):
        raise ValueError("invalid_source_row")
    row = etl.transform_row(table, event["tenant"], operation["source_row"])
    columns = [column.target for column in table.columns]
    column_sql = ", ".join(etl.quote(column) for column in columns)
    placeholders = ", ".join("?" for _ in columns)
    update_columns = [column for column in columns if column not in table.primary_key]
    updates = ", ".join(f"{etl.quote(column)} = excluded.{etl.quote(column)}" for column in update_columns)
    conflict = ", ".join(etl.quote(column) for column in table.primary_key)
    conflict_action = f"DO UPDATE SET {updates}" if updates else "DO NOTHING"
    sql = (
        f"INSERT INTO {etl.quote(table.name)} ({column_sql}) VALUES ({placeholders}) "
        f"ON CONFLICT ({conflict}) {conflict_action}"
    )
    target.execute(sql, tuple(row[column] for column in columns))
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
    if max_events <= 0:
        raise CutoverError("replay max_events must be positive")
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
        if state["phase"] not in {"dual_write", "rolled_back"} or not state["mirror_enabled"]:
            raise CutoverError("replay requires the controlled mirror phase")
        if state["replay_paused"]:
            raise CutoverError("replay is paused by an audited operator control")
        events = state_db.execute(
            """SELECT * FROM captured_events
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
        for row in events:
            if row["sequence"] != next_sequence:
                break
            ledger = target.execute(
                "SELECT event_digest FROM _frame_change_ledger WHERE event_id = ?", (row["event_id"],)
            ).fetchone()
            if ledger:
                if ledger["event_digest"] != row["event_digest"]:
                    raise CutoverError("target replay ledger has a conflicting event digest")
                state_db.execute("BEGIN IMMEDIATE")
                state_db.execute(
                    "UPDATE captured_events SET state = 'applied', applied_at_ms = ? WHERE event_id = ? AND state = 'pending'",
                    (applied_at_ms, row["event_id"]),
                )
                state_db.commit()
                recovered += 1
                next_sequence += 1
                continue
            try:
                event = etl.strict_json(row["payload_json"])
                target.execute("BEGIN IMMEDIATE")
                _apply_event(target, config, event, scope[0], row["event_digest"], applied_at_ms)
                target.commit()
            except (KeyError, TypeError, ValueError, json.JSONDecodeError, sqlite3.Error) as error:
                target.rollback()
                reason = str(error).split(":", maxsplit=1)[0]
                if not etl.SAFE_LABEL.fullmatch(reason):
                    reason = "invalid_event"
                state_db.execute("BEGIN IMMEDIATE")
                state_db.execute(
                    "UPDATE captured_events SET state = 'dead_letter', reason_code = ? WHERE event_id = ? AND state = 'pending'",
                    (reason, row["event_id"]),
                )
                state_db.commit()
                dead_lettered += 1
                break
            if inject_after_target_commit:
                raise InjectedReplayInterruption
            state_db.execute("BEGIN IMMEDIATE")
            state_db.execute(
                "UPDATE captured_events SET state = 'applied', applied_at_ms = ? WHERE event_id = ? AND state = 'pending'",
                (applied_at_ms, row["event_id"]),
            )
            state_db.commit()
            applied += 1
            next_sequence += 1
    finally:
        target.close()
        state_db.close()
    return {"applied": applied, "recovered_after_commit": recovered, "dead_lettered": dead_lettered}


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
        return {
            "tenant_digest": scope[0],
            "domain": domain,
            "phase": state["phase"],
            "writer": state["writer"],
            "mirror_enabled": bool(state["mirror_enabled"]),
            "replay_paused": bool(state["replay_paused"]),
            "epoch": state["epoch"],
            "pending_events": counts.get("pending", 0),
            "dead_letter_events": counts.get("dead_letter", 0),
            "applied_events": counts.get("applied", 0),
            "oldest_pending_lag_ms": 0 if oldest is None else max(0, now_ms - oldest),
            "shadow_classifications": shadow,
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
        else:
            result = status(args.state, config, args.tenant, args.domain, args.now_ms)
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
