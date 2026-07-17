#!/usr/bin/env python3
"""Provider-free SQLite conformance for the legacy-operation D1 journal.

This exercises only synthetic digests. It proves the SQL transaction and
postcondition expected by ``D1LegacyOperationExecutionPortV1``; the reported
static-adapter inventory is separately source/test guarded and is not a claim
that this SQL exercise promoted it or stands in for deployed D1 evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sqlite3
import tempfile
import threading
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATION = ROOT / "apps/control-plane/migrations/0026_legacy_api_execution.sql"
QUERY_ROOT = ROOT / "apps/control-plane/queries/api_workflow"
CLAIM = QUERY_ROOT / "legacy_execution_claim.sql"
INTENT = QUERY_ROOT / "legacy_execution_intent.sql"
COMPLETE = QUERY_ROOT / "legacy_execution_complete.sql"
AUDIT = QUERY_ROOT / "legacy_execution_audit.sql"
LOAD = QUERY_ROOT / "legacy_execution_load.sql"

OPERATION_ID = "cap-v1-0000000000000000"
AUDIT_ACTION = "legacy.synthetic-conformance"
RESPONSE_STATUS = 202
STATIC_SEMANTIC_ADAPTER_ID = "cap-v1-05b6ba3f76daac22"


def digest(seed: str) -> str:
    return hashlib.sha256(f"frame-synthetic:{seed}".encode()).hexdigest()


def digest_parts(domain: str, parts: tuple[str, ...]) -> str:
    value = hashlib.sha256()
    value.update(domain.encode())
    for part in parts:
        value.update(b"\0")
        value.update(part.encode())
    return value.hexdigest()


def connection(path: Path) -> sqlite3.Connection:
    database = sqlite3.connect(path, timeout=10, isolation_level=None)
    database.execute("PRAGMA busy_timeout = 10000")
    database.execute("PRAGMA foreign_keys = ON")
    return database


@dataclass(frozen=True)
class Execution:
    scope_digest: str
    key_digest: str
    fingerprint: str
    reservation: str
    result_digest: str
    audit_id: str
    correlation_digest: str
    occurred_at_ms: int


def make_execution(
    *,
    scope: str,
    raw_key: str,
    fingerprint_seed: str,
    reservation_seed: str,
    occurred_at_ms: int,
) -> Execution:
    key_digest = digest_parts(
        "legacy-idempotency-v1", (scope, OPERATION_ID, raw_key)
    )
    reservation = digest(reservation_seed)
    result_digest = digest_parts(
        "legacy-accepted-result-v1",
        (scope, OPERATION_ID, key_digest, digest(fingerprint_seed)),
    )
    return Execution(
        scope_digest=scope,
        key_digest=key_digest,
        fingerprint=digest(fingerprint_seed),
        reservation=reservation,
        result_digest=result_digest,
        audit_id=digest_parts("legacy-audit-v1", (reservation, result_digest)),
        correlation_digest=digest(f"correlation-{reservation_seed}"),
        occurred_at_ms=occurred_at_ms,
    )


def query_text() -> dict[str, str]:
    return {
        "claim": CLAIM.read_text(encoding="utf-8"),
        "intent": INTENT.read_text(encoding="utf-8"),
        "complete": COMPLETE.read_text(encoding="utf-8"),
        "audit": AUDIT.read_text(encoding="utf-8"),
        "load": LOAD.read_text(encoding="utf-8"),
    }


def execute_batch(
    database: sqlite3.Connection,
    queries: dict[str, str],
    execution: Execution,
) -> list[int]:
    common = (
        execution.scope_digest,
        OPERATION_ID,
        execution.key_digest,
        execution.reservation,
        execution.fingerprint,
    )
    database.execute("BEGIN IMMEDIATE")
    try:
        counts = [
            len(
                database.execute(
                    queries["claim"],
                    (
                        execution.scope_digest,
                        OPERATION_ID,
                        execution.key_digest,
                        execution.fingerprint,
                        execution.reservation,
                        execution.occurred_at_ms,
                    ),
                ).fetchall()
            ),
            len(
                database.execute(
                    queries["intent"],
                    (*common, execution.occurred_at_ms),
                ).fetchall()
            ),
            len(
                database.execute(
                    queries["complete"],
                    (
                        *common,
                        RESPONSE_STATUS,
                        execution.result_digest,
                        execution.occurred_at_ms,
                    ),
                ).fetchall()
            ),
            len(
                database.execute(
                    queries["audit"],
                    (
                        *common,
                        RESPONSE_STATUS,
                        execution.result_digest,
                        execution.occurred_at_ms,
                        execution.audit_id,
                        AUDIT_ACTION,
                        execution.correlation_digest,
                    ),
                ).fetchall()
            ),
        ]
    except Exception:
        database.execute("ROLLBACK")
        raise
    database.execute("COMMIT")
    return counts


def load_row(
    database: sqlite3.Connection,
    queries: dict[str, str],
    execution: Execution,
) -> tuple[object, ...] | None:
    return database.execute(
        queries["load"],
        (execution.scope_digest, OPERATION_ID, execution.key_digest),
    ).fetchone()


def expect_integrity(database: sqlite3.Connection, sql: str, values: tuple[object, ...]) -> None:
    try:
        database.execute(sql, values)
    except sqlite3.IntegrityError:
        return
    raise AssertionError("invalid or mutable legacy execution row was accepted")


def assert_query_guards(queries: dict[str, str]) -> None:
    if not queries["claim"].startswith("INSERT OR IGNORE"):
        raise AssertionError("claim query lost INSERT OR IGNORE")
    if "RETURNING reservation_digest" not in queries["claim"]:
        raise AssertionError("claim query lost its atomic-returning contract")
    for name in ("intent", "complete", "audit"):
        query = queries[name]
        if "reservation_digest = ?4" not in query:
            raise AssertionError(f"{name} query is not fenced by the reservation")
        if "request_fingerprint = ?5" not in query:
            raise AssertionError(f"{name} query is not bound to the request")
    if "state = 'pending'" not in queries["complete"]:
        raise AssertionError("completion query lost the pending-state guard")
    if "state = 'complete'" not in queries["audit"]:
        raise AssertionError("audit query lost the completed-state guard")
    if "LEFT JOIN legacy_api_execution_intents_v1" not in queries["load"]:
        raise AssertionError("load query lost intent postcondition evidence")
    if "LEFT JOIN legacy_api_execution_audit_v1" not in queries["load"]:
        raise AssertionError("load query lost audit postcondition evidence")


def run() -> dict[str, object]:
    migration = MIGRATION.read_text(encoding="utf-8")
    queries = query_text()
    assert_query_guards(queries)

    with tempfile.TemporaryDirectory(prefix="frame-legacy-execution-") as directory:
        path = Path(directory) / "legacy-execution.sqlite3"
        database = connection(path)
        database.executescript(migration)
        database.execute("PRAGMA journal_mode = WAL")
        database.close()

        scope = digest("scope-a")
        contender_a = make_execution(
            scope=scope,
            raw_key="synthetic-key",
            fingerprint_seed="request-a",
            reservation_seed="reservation-a",
            occurred_at_ms=1_000,
        )
        contender_b = make_execution(
            scope=scope,
            raw_key="synthetic-key",
            fingerprint_seed="request-a",
            reservation_seed="reservation-b",
            occurred_at_ms=1_001,
        )

        barrier = threading.Barrier(2)
        race_outcomes: list[list[int]] = []
        race_failures: list[str] = []
        outcome_lock = threading.Lock()

        def contender(execution: Execution) -> None:
            candidate = connection(path)
            try:
                barrier.wait(timeout=5)
                outcome = execute_batch(candidate, queries, execution)
                with outcome_lock:
                    race_outcomes.append(outcome)
            except Exception as error:  # pragma: no cover - reported below
                with outcome_lock:
                    race_failures.append(type(error).__name__)
            finally:
                candidate.close()

        threads = [
            threading.Thread(target=contender, args=(contender_a,)),
            threading.Thread(target=contender, args=(contender_b,)),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=15)
        expected_race = sorted(([0, 0, 0, 0], [1, 1, 1, 1]))
        if (
            any(thread.is_alive() for thread in threads)
            or race_failures
            or sorted(race_outcomes) != expected_race
        ):
            raise AssertionError(
                "atomic journal race failed: "
                f"outcomes={sorted(race_outcomes)} failures={race_failures}"
            )

        database = connection(path)
        rows = database.execute(
            "SELECT COUNT(*) FROM legacy_api_execution_operations_v1"
        ).fetchone()
        intents = database.execute(
            "SELECT COUNT(*) FROM legacy_api_execution_intents_v1"
        ).fetchone()
        audits = database.execute(
            "SELECT COUNT(*) FROM legacy_api_execution_audit_v1"
        ).fetchone()
        if rows != (1,) or intents != (1,) or audits != (1,):
            raise AssertionError("winning batch did not persist one complete journal")

        winning = load_row(database, queries, contender_a)
        if winning is None:
            raise AssertionError("winning journal cannot be reloaded")
        if winning[2:] != (
            "complete",
            RESPONSE_STATUS,
            winning[4],
            winning[1],
            winning[1],
        ):
            raise AssertionError("complete journal lost its intent/audit postcondition")
        winning_reservation = str(winning[1])

        # A process restart sees an exact replay and cannot create another
        # operation, intent, audit, or result.
        database.close()
        database = connection(path)
        replay = make_execution(
            scope=scope,
            raw_key="synthetic-key",
            fingerprint_seed="request-a",
            reservation_seed="reservation-after-restart",
            occurred_at_ms=1_002,
        )
        if execute_batch(database, queries, replay) != [0, 0, 0, 0]:
            raise AssertionError("exact replay mutated the durable journal")
        replay_row = load_row(database, queries, replay)
        if replay_row is None or replay_row[1] != winning_reservation:
            raise AssertionError("exact replay did not return the original reservation")

        # Reusing the scoped key with another request is persisted as neither
        # a new operation nor a partial intent. The runtime maps the loaded
        # fingerprint mismatch to a stable conflict.
        conflict = make_execution(
            scope=scope,
            raw_key="synthetic-key",
            fingerprint_seed="request-conflict",
            reservation_seed="reservation-conflict",
            occurred_at_ms=1_003,
        )
        if execute_batch(database, queries, conflict) != [0, 0, 0, 0]:
            raise AssertionError("conflicting key reuse mutated the durable journal")
        conflict_row = load_row(database, queries, conflict)
        if conflict_row is None or conflict_row[0] == conflict.fingerprint:
            raise AssertionError("conflicting key reuse is not externally distinguishable")

        # Even individually invoked loser statements are fenced by the
        # winning reservation and cannot create a partial journal.
        loser_common = (
            replay.scope_digest,
            OPERATION_ID,
            replay.key_digest,
            replay.reservation,
            replay.fingerprint,
        )
        guarded_counts = [
            len(
                database.execute(
                    queries["intent"], (*loser_common, replay.occurred_at_ms)
                ).fetchall()
            ),
            len(
                database.execute(
                    queries["complete"],
                    (
                        *loser_common,
                        RESPONSE_STATUS,
                        replay.result_digest,
                        replay.occurred_at_ms,
                    ),
                ).fetchall()
            ),
            len(
                database.execute(
                    queries["audit"],
                    (
                        *loser_common,
                        RESPONSE_STATUS,
                        replay.result_digest,
                        replay.occurred_at_ms,
                        replay.audit_id,
                        AUDIT_ACTION,
                        replay.correlation_digest,
                    ),
                ).fetchall()
            ),
        ]
        if guarded_counts != [0, 0, 0]:
            raise AssertionError("losing reservation produced a partial journal")

        # The same synthetic raw key in a second scope hashes differently and
        # can establish an independent operation.
        other_scope = digest("scope-b")
        scoped = make_execution(
            scope=other_scope,
            raw_key="synthetic-key",
            fingerprint_seed="request-a",
            reservation_seed="reservation-other-scope",
            occurred_at_ms=1_004,
        )
        if scoped.key_digest == contender_a.key_digest:
            raise AssertionError("idempotency digest is not scope-bound")
        if execute_batch(database, queries, scoped) != [1, 1, 1, 1]:
            raise AssertionError("independent scope did not establish a complete journal")

        expect_integrity(
            database,
            "INSERT INTO legacy_api_execution_operations_v1 VALUES "
            "(?1, ?2, ?3, ?4, ?5, 'pending', NULL, NULL, 1, NULL)",
            ("not-a-digest", OPERATION_ID, digest("key"), digest("request"), digest("reservation")),
        )
        expect_integrity(
            database,
            "UPDATE legacy_api_execution_operations_v1 SET result_digest=?1 "
            "WHERE scope_digest=?2 AND operation_id=?3 AND idempotency_key_digest=?4",
            (digest("replacement"), scope, OPERATION_ID, contender_a.key_digest),
        )
        expect_integrity(
            database,
            "DELETE FROM legacy_api_execution_intents_v1 WHERE reservation_digest=?1",
            (winning_reservation,),
        )
        expect_integrity(
            database,
            "DELETE FROM legacy_api_execution_audit_v1 WHERE reservation_digest=?1",
            (winning_reservation,),
        )

        final_counts = database.execute(
            "SELECT "
            "(SELECT COUNT(*) FROM legacy_api_execution_operations_v1), "
            "(SELECT COUNT(*) FROM legacy_api_execution_intents_v1), "
            "(SELECT COUNT(*) FROM legacy_api_execution_audit_v1)"
        ).fetchone()
        database.close()
        if final_counts != (2, 2, 2):
            raise AssertionError("constraint probes changed the complete journals")

    return {
        "schema_version": "frame.legacy-api-execution-sqlite-conformance.v1",
        "provider": "local_sqlite",
        "synthetic_only": True,
        "semantic_adapters_enabled": 1,
        "semantic_adapter_operation_ids": [STATIC_SEMANTIC_ADAPTER_ID],
        "durable_semantic_adapters_enabled": 0,
        "inventory_endpoint_success_promoted": 1,
        "race_contenders": 2,
        "race_complete": 1,
        "race_replays": 1,
        "restart_exact_replay": True,
        "conflicting_key_reuse_rejected": True,
        "loser_partial_mutations": 0,
        "scope_bound_idempotency": True,
        "immutable_constraint_cases": 4,
        "complete_journals": 2,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    report = run()
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(
        "Legacy API D1 journal conformance passed: one of two racers completed; "
        "restart replay, conflict, reservation fencing, scoping, and immutability passed."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
