#!/usr/bin/env python3
"""Provider-free SQLite conformance for the D1 webhook replay authority."""

from __future__ import annotations

import argparse
import json
import sqlite3
import tempfile
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATION = ROOT / "apps/control-plane/migrations/0015_api_workflow_replay.sql"
CLAIM = ROOT / "apps/control-plane/queries/api_workflow/webhook_replay_claim.sql"
PRUNE = ROOT / "apps/control-plane/queries/api_workflow/webhook_replay_prune.sql"


def digest(byte: str) -> str:
    return byte * 64


def connection(path: Path) -> sqlite3.Connection:
    database = sqlite3.connect(path, timeout=10, isolation_level=None)
    database.execute("PRAGMA busy_timeout = 10000")
    return database


def expect_integrity(database: sqlite3.Connection, sql: str, values: tuple[object, ...]) -> None:
    try:
        database.execute(sql, values)
    except sqlite3.IntegrityError:
        return
    raise AssertionError("invalid replay row was accepted")


def run() -> dict[str, object]:
    migration = MIGRATION.read_text(encoding="utf-8")
    claim_sql = CLAIM.read_text(encoding="utf-8")
    prune_sql = PRUNE.read_text(encoding="utf-8")
    if "VALUES (?1, ?2, ?3)" not in claim_sql or "RETURNING replay_digest" not in claim_sql:
        raise AssertionError("claim query lost its bound atomic-returning contract")
    if "expires_at_ms < ?1" not in prune_sql or "LIMIT ?2" not in prune_sql:
        raise AssertionError("prune query lost its expiry/bound contract")

    with tempfile.TemporaryDirectory(prefix="frame-api-replay-") as directory:
        path = Path(directory) / "replay.sqlite3"
        database = connection(path)
        database.executescript(migration)
        database.execute("PRAGMA journal_mode = WAL")
        database.close()

        barrier = threading.Barrier(2)
        outcomes: list[int] = []
        failures: list[str] = []
        outcome_lock = threading.Lock()

        def contender() -> None:
            candidate = connection(path)
            try:
                barrier.wait(timeout=5)
                rows = candidate.execute(claim_sql, (digest("a"), 1_000, 2_000)).fetchall()
                with outcome_lock:
                    outcomes.append(len(rows))
            except Exception as error:  # pragma: no cover - reported below
                with outcome_lock:
                    failures.append(type(error).__name__)
            finally:
                candidate.close()

        threads = [threading.Thread(target=contender) for _ in range(2)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=15)
        if any(thread.is_alive() for thread in threads) or failures or sorted(outcomes) != [0, 1]:
            raise AssertionError(
                f"atomic race failed: outcomes={sorted(outcomes)} failures={failures}"
            )

        database = connection(path)
        if database.execute(
            "SELECT COUNT(*) FROM api_webhook_replay_claims_v1 WHERE replay_digest=?1",
            (digest("a"),),
        ).fetchone() != (1,):
            raise AssertionError("atomic race did not persist exactly one claim")

        # A process restart sees the durable duplicate and performs no effect.
        if database.execute(claim_sql, (digest("a"), 1_001, 2_001)).fetchall():
            raise AssertionError("durable duplicate claim was accepted after restart")

        expect_integrity(
            database,
            "INSERT INTO api_webhook_replay_claims_v1 VALUES (?1, ?2, ?3)",
            ("not-a-digest", 1_000, 2_000),
        )
        expect_integrity(
            database,
            "INSERT INTO api_webhook_replay_claims_v1 VALUES (?1, ?2, ?3)",
            (digest("b"), 2_000, 2_000),
        )
        expect_integrity(
            database,
            "INSERT INTO api_webhook_replay_claims_v1 VALUES (?1, ?2, ?3)",
            (digest("c"), 1_000, 1_801_001),
        )
        try:
            database.execute(
                "UPDATE api_webhook_replay_claims_v1 SET expires_at_ms=3000 "
                "WHERE replay_digest=?1",
                (digest("a"),),
            )
        except sqlite3.IntegrityError:
            pass
        else:
            raise AssertionError("immutable replay claim was updated")

        for index, value in enumerate(("b", "c", "d"), start=1):
            rows = database.execute(
                claim_sql,
                (digest(value), 1_000 + index, 1_500 + index),
            ).fetchall()
            if len(rows) != 1:
                raise AssertionError("fixture replay claim was not inserted")
        database.execute(claim_sql, (digest("e"), 1_500, 4_000)).fetchall()

        if database.execute(prune_sql, (1_501, 10)).fetchall():
            raise AssertionError("prune removed a claim at its inclusive expiry boundary")
        first_prune = database.execute(prune_sql, (2_000, 2)).fetchall()
        second_prune = database.execute(prune_sql, (2_000, 2)).fetchall()
        if len(first_prune) != 2 or len(second_prune) != 1:
            raise AssertionError("bounded expiry cleanup returned an unexpected row count")
        if database.execute(
            "SELECT COUNT(*) FROM api_webhook_replay_claims_v1 WHERE replay_digest=?1",
            (digest("e"),),
        ).fetchone() != (1,):
            raise AssertionError("cleanup removed an unexpired claim")
        database.close()

    return {
        "schema_version": "frame.api-workflow-d1-conformance.v1",
        "provider": "local_sqlite",
        "synthetic_only": True,
        "race_contenders": 2,
        "race_claimed": 1,
        "race_duplicates": 1,
        "restart_duplicate_rejected": True,
        "constraint_cases": 4,
        "bounded_prune_batches": [2, 1],
        "unexpired_preserved": True,
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
        "API workflow D1 replay conformance passed: "
        "one of two racers claimed; restart replay rejected; expiry cleanup bounded."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
