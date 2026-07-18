#!/usr/bin/env python3
"""Exercise the ciphertext-only Worker auth provider-handoff invariants."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sqlite3


ROOT = pathlib.Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
NOW = 1_000_000
DELIVERY = "00000000-0000-7000-8000-000000000001"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"worker auth sqlite conformance: {message}")


def rejected(connection: sqlite3.Connection, sql: str, parameters: tuple[object, ...]) -> bool:
    try:
        connection.execute(sql, parameters)
    except sqlite3.IntegrityError:
        return True
    return False


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path)
    args = parser.parse_args()

    connection = sqlite3.connect(":memory:", isolation_level=None)
    connection.execute("PRAGMA foreign_keys = ON")
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))

    payload = "ab" * 1071
    digest = hashlib.sha256(bytes.fromhex(payload)).hexdigest()
    require(
        rejected(
            connection,
            "INSERT INTO auth_delivery_provider_handoffs_v1("
            "delivery_id,payload_hex,payload_sha256,state,provider_attempt,"
            "provider_receipt_digest,created_at_ms,updated_at_ms) "
            "VALUES (?,?,?,'delivered',1,?,?,?)",
            (
                "00000000-0000-7000-8000-000000000010",
                payload,
                digest,
                "ef" * 32,
                NOW,
                NOW,
            ),
        ),
        "a forged delivered handoff was accepted on insert",
    )
    require(
        rejected(
            connection,
            "INSERT INTO auth_delivery_provider_handoffs_v1("
            "delivery_id,payload_hex,payload_sha256,state,provider_attempt,"
            "provider_lease_id,provider_lease_expires_at_ms,created_at_ms,updated_at_ms) "
            "VALUES (?,?,?,'delivering',1,?,?,?,?)",
            (
                "00000000-0000-7000-8000-000000000011",
                payload,
                digest,
                "00000000-0000-7000-8000-000000000012",
                NOW + 60_000,
                NOW,
                NOW,
            ),
        ),
        "a forged delivering handoff was accepted on insert",
    )
    insert = (
        "INSERT OR IGNORE INTO auth_delivery_provider_handoffs_v1("
        "delivery_id,payload_hex,payload_sha256,state,provider_attempt,created_at_ms,updated_at_ms) "
        "VALUES (?,?,?,'pending',0,?,?)"
    )
    connection.execute(insert, (DELIVERY, payload, digest, NOW, NOW))
    connection.execute(insert, (DELIVERY, payload, digest, NOW, NOW))
    require(
        connection.execute(
            "SELECT COUNT(*) FROM auth_delivery_provider_handoffs_v1 WHERE delivery_id=?",
            (DELIVERY,),
        ).fetchone()
        == (1,),
        "crash replay duplicated a handoff",
    )

    conflicting_payload = "cd" * 1071
    conflicting_digest = hashlib.sha256(bytes.fromhex(conflicting_payload)).hexdigest()
    connection.execute(
        insert,
        (DELIVERY, conflicting_payload, conflicting_digest, NOW, NOW),
    )
    require(
        connection.execute(
            "SELECT payload_sha256 FROM auth_delivery_provider_handoffs_v1 WHERE delivery_id=?",
            (DELIVERY,),
        ).fetchone()
        == (digest,),
        "delivery-ID collision replaced immutable ciphertext",
    )
    require(
        rejected(
            connection,
            "UPDATE auth_delivery_provider_handoffs_v1 SET payload_hex=? WHERE delivery_id=?",
            (conflicting_payload, DELIVERY),
        ),
        "ciphertext was mutable after handoff",
    )
    require(
        rejected(
            connection,
            "UPDATE auth_delivery_provider_handoffs_v1 SET state='delivered',updated_at_ms=? "
            "WHERE delivery_id=?",
            (NOW + 1, DELIVERY),
        ),
        "pending handoff skipped the fenced delivery state",
    )

    lease = "00000000-0000-7000-8000-000000000002"
    require(
        rejected(
            connection,
            "UPDATE auth_delivery_provider_handoffs_v1 SET state='delivering',"
            "provider_lease_id=?,provider_lease_expires_at_ms=?,updated_at_ms=? "
            "WHERE delivery_id=?",
            (lease, NOW + 60_000, NOW + 1, DELIVERY),
        ),
        "pending claim did not require an exact attempt increment",
    )
    connection.execute(
        "UPDATE auth_delivery_provider_handoffs_v1 SET state='delivering',provider_attempt=1,"
        "provider_lease_id=?,provider_lease_expires_at_ms=?,updated_at_ms=? WHERE delivery_id=?",
        (lease, NOW + 60_000, NOW + 1, DELIVERY),
    )
    require(
        rejected(
            connection,
            "UPDATE auth_delivery_provider_handoffs_v1 SET provider_attempt=3,updated_at_ms=? "
            "WHERE delivery_id=?",
            (NOW + 2, DELIVERY),
        ),
        "provider attempt fencing allowed a skipped attempt",
    )
    replacement_lease = "00000000-0000-7000-8000-000000000003"
    require(
        rejected(
            connection,
            "UPDATE auth_delivery_provider_handoffs_v1 SET provider_attempt=2,"
            "provider_lease_id=?,provider_lease_expires_at_ms=?,updated_at_ms=? "
            "WHERE delivery_id=?",
            (replacement_lease, NOW + 120_000, NOW + 2, DELIVERY),
        ),
        "an unexpired provider lease was replaced",
    )
    require(
        rejected(
            connection,
            "UPDATE auth_delivery_provider_handoffs_v1 SET state='pending',"
            "provider_lease_id=NULL,provider_lease_expires_at_ms=NULL,next_attempt_at_ms=?,"
            "last_error_class='provider_unavailable',updated_at_ms=? WHERE delivery_id=?",
            (NOW + 120_000, NOW + 2, DELIVERY),
        ),
        "an unexpired provider lease was released for retry",
    )
    require(
        rejected(
            connection,
            "UPDATE auth_delivery_provider_handoffs_v1 SET state='delivered',"
            "provider_lease_id=NULL,provider_lease_expires_at_ms=NULL,updated_at_ms=? "
            "WHERE delivery_id=?",
            (NOW + 2, DELIVERY),
        ),
        "delivery completed without an authenticated provider receipt",
    )
    receipt = "ef" * 32
    connection.execute(
        "UPDATE auth_delivery_provider_handoffs_v1 SET state='delivered',provider_lease_id=NULL,"
        "provider_lease_expires_at_ms=NULL,provider_receipt_digest=?,updated_at_ms=? "
        "WHERE delivery_id=? AND state='delivering' AND provider_lease_id=?",
        (receipt, NOW + 2, DELIVERY, lease),
    )
    require(
        connection.execute(
            "SELECT state,provider_attempt,provider_receipt_digest "
            "FROM auth_delivery_provider_handoffs_v1 WHERE delivery_id=?",
            (DELIVERY,),
        ).fetchone()
        == ("delivered", 1, receipt),
        "fenced provider receipt was not retained",
    )
    require(
        rejected(
            connection,
            "UPDATE auth_delivery_provider_handoffs_v1 SET state='pending',updated_at_ms=? "
            "WHERE delivery_id=?",
            (NOW + 3, DELIVERY),
        ),
        "terminal handoff was reopened",
    )
    require(
        rejected(
            connection,
            "UPDATE auth_delivery_provider_handoffs_v1 SET provider_receipt_digest=?,"
            "updated_at_ms=? WHERE delivery_id=?",
            ("aa" * 32, NOW + 4, DELIVERY),
        ),
        "terminal provider receipt was mutable",
    )

    report = {
        "schema": "frame.worker-auth-sqlite-conformance.v1",
        "migration": "0029_worker_auth_delivery_handoff.sql",
        "ciphertext_bytes": 1071,
        "cases": {
            "forged_delivered_insert_rejected": True,
            "forged_delivering_insert_rejected": True,
            "crash_replay_deduplicated": True,
            "delivery_id_collision_detected_by_digest": True,
            "payload_immutable": True,
            "claim_requires_exact_attempt_increment": True,
            "unexpired_lease_takeover_rejected": True,
            "unexpired_retry_rejected": True,
            "delivery_requires_receipt": True,
            "state_and_attempt_fenced": True,
            "terminal_state_immutable": True,
            "terminal_receipt_immutable": True,
        },
        "provider_execution": "protected_not_executed",
    }
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print("worker auth sqlite conformance passed: 12 ciphertext handoff cases")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
