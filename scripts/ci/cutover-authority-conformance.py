#!/usr/bin/env python3
"""Credential-free SQLite proof for the scoped D1 cutover authority contract."""

from __future__ import annotations

import json
import pathlib
import sqlite3
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
QUERIES = ROOT / "apps" / "control-plane" / "queries" / "cutover"
ZERO = "0" * 64


def apply_migrations() -> sqlite3.Connection:
    database = sqlite3.connect(":memory:")
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        database.executescript(migration.read_text(encoding="utf-8"))
    return database


def expect_integrity_error(callable_object, *args, **kwargs) -> None:
    try:
        callable_object(*args, **kwargs)
    except sqlite3.IntegrityError:
        return
    raise AssertionError("expected the scoped authority contract to fail closed")


def seed_scope(database: sqlite3.Connection, tenant: str, at_ms: int) -> None:
    database.execute(
        """INSERT INTO cutover_authority_scopes(
             tenant_id, domain, phase, writer, mirror_enabled, replay_paused,
             epoch, audit_head, rollback_ready, phase_started_at_ms, updated_at_ms
           ) VALUES (?, 'metadata', 'legacy_authoritative', 'legacy', 0, 0,
                     0, ?, 0, ?, ?)""",
        (tenant, ZERO, at_ms, at_ms),
    )


def transition(
    database: sqlite3.Connection,
    tenant: str,
    *,
    from_phase: str,
    to_phase: str,
    from_epoch: int,
    previous_hash: str,
    audit_hash: str,
    at_ms: int,
) -> None:
    database.execute(
        """INSERT INTO cutover_authority_audit(
             audit_hash, previous_hash, tenant_id, domain, action,
             from_phase, to_phase, from_epoch, to_epoch,
             operator_digest, evidence_digest, occurred_at_ms
           ) VALUES (?, ?, ?, 'metadata', 'transition', ?, ?, ?, ?, ?, ?, ?)""",
        (
            audit_hash,
            previous_hash,
            tenant,
            from_phase,
            to_phase,
            from_epoch,
            from_epoch + 1,
            "a" * 64,
            "b" * 64,
            at_ms,
        ),
    )
    writer = "d1" if to_phase in {"d1_authoritative", "finalized"} else "legacy"
    mirror = int(to_phase in {"dual_write", "d1_authoritative", "rolled_back"})
    rollback = int(to_phase == "d1_authoritative")
    database.execute(
        """UPDATE cutover_authority_scopes
           SET phase = ?, writer = ?, mirror_enabled = ?, replay_paused = 0,
               epoch = ?, phase_epoch = ?, audit_head = ?, rollback_ready = ?,
               phase_started_at_ms = ?, updated_at_ms = ?
           WHERE tenant_id = ? AND domain = 'metadata' AND epoch = ?""",
        (
            to_phase,
            writer,
            mirror,
            from_epoch + 1,
            from_epoch + 1,
            audit_hash,
            rollback,
            at_ms,
            at_ms,
            tenant,
            from_epoch,
        ),
    )


def seed_policy(database: sqlite3.Connection, tenant: str, at_ms: int) -> None:
    database.execute(
        """INSERT INTO cutover_slo_config(
             tenant_id, domain, shadow_window_ms, minimum_shadow_observations,
             max_pending_lag_ms, max_shadow_mismatches, max_dead_letter_events,
             max_contention_events, approved_by_digest, updated_at_ms
           ) VALUES (?, 'metadata', 50, 1, 100, 0, 0, 1, ?, ?)""",
        (tenant, "c" * 64, at_ms),
    )
    database.execute(
        """INSERT INTO cutover_shadow_query_requirements(
             tenant_id, domain, query_class, normalization_digest,
             approved_by_digest, created_at_ms
           ) VALUES (?, 'metadata', 'video_list', ?, ?, ?)""",
        (tenant, "d" * 64, "c" * 64, at_ms),
    )


def assertion(
    database: sqlite3.Connection,
    assertion_id: str,
    tenant: str,
    writer: str,
    epoch: int,
    at_ms: int,
) -> None:
    database.execute(
        (QUERIES / "writer_assert.sql").read_text(encoding="utf-8"),
        (assertion_id, tenant, "metadata", writer, epoch, at_ms),
    )


def execute_atomic(database: sqlite3.Connection, statements: list[tuple[str, tuple]]) -> None:
    database.execute("BEGIN")
    try:
        for sql, bindings in statements:
            database.execute(sql, bindings)
        database.commit()
    except BaseException:
        database.rollback()
        raise


def authority_transition_batch(
    database: sqlite3.Connection,
    tenant: str,
    *,
    from_phase: str,
    to_phase: str,
    from_epoch: int,
    previous_hash: str,
    audit_hash: str,
    at_ms: int,
    require_health: bool,
    require_drained: bool,
) -> None:
    writer = "d1" if to_phase == "d1_authoritative" else "legacy"
    mirror = int(to_phase in {"dual_write", "d1_authoritative", "rolled_back"})
    gate_id = f"{audit_hash}:transition"
    postcondition_id = f"{audit_hash}:postcondition"
    execute_atomic(
        database,
        [
            (
                (QUERIES / "transition_assert.sql").read_text(encoding="utf-8"),
                (
                    gate_id,
                    tenant,
                    "metadata",
                    from_epoch,
                    previous_hash,
                    to_phase,
                    at_ms,
                    int(to_phase != "rolled_back"),
                    int(require_health),
                    int(require_drained),
                ),
            ),
            (
                (QUERIES / "audit_insert.sql").read_text(encoding="utf-8"),
                (
                    audit_hash,
                    previous_hash,
                    tenant,
                    "metadata",
                    "transition",
                    from_phase,
                    to_phase,
                    from_epoch,
                    from_epoch + 1,
                    "a" * 64,
                    "b" * 64,
                    at_ms,
                ),
            ),
            (
                (QUERIES / "scope_transition.sql").read_text(encoding="utf-8"),
                (
                    tenant,
                    "metadata",
                    from_phase,
                    from_epoch,
                    previous_hash,
                    to_phase,
                    writer,
                    mirror,
                    audit_hash,
                    int(to_phase == "d1_authoritative"),
                    at_ms,
                ),
            ),
            (
                (QUERIES / "state_postcondition.sql").read_text(encoding="utf-8"),
                (
                    postcondition_id,
                    tenant,
                    "metadata",
                    to_phase,
                    writer,
                    0,
                    from_epoch + 1,
                    audit_hash,
                    at_ms,
                    from_epoch + 1,
                ),
            ),
            (
                (QUERIES / "assertion_cleanup.sql").read_text(encoding="utf-8"),
                (gate_id,),
            ),
            (
                (QUERIES / "assertion_cleanup.sql").read_text(encoding="utf-8"),
                (postcondition_id,),
            ),
        ],
    )


def replay_control_batch(
    database: sqlite3.Connection,
    tenant: str,
    *,
    phase: str,
    writer: str,
    action: str,
    from_epoch: int,
    previous_hash: str,
    audit_hash: str,
    at_ms: int,
    phase_epoch: int,
) -> None:
    paused = int(action == "pause")
    gate_id = f"{audit_hash}:{action}"
    postcondition_id = f"{audit_hash}:postcondition"
    execute_atomic(
        database,
        [
            (
                (QUERIES / "control_assert.sql").read_text(encoding="utf-8"),
                (
                    gate_id,
                    tenant,
                    "metadata",
                    from_epoch,
                    previous_hash,
                    action,
                    at_ms,
                    int(action == "resume"),
                ),
            ),
            (
                (QUERIES / "audit_insert.sql").read_text(encoding="utf-8"),
                (
                    audit_hash,
                    previous_hash,
                    tenant,
                    "metadata",
                    action,
                    phase,
                    phase,
                    from_epoch,
                    from_epoch + 1,
                    "a" * 64,
                    "b" * 64,
                    at_ms,
                ),
            ),
            (
                (QUERIES / "scope_control.sql").read_text(encoding="utf-8"),
                (
                    tenant,
                    "metadata",
                    phase,
                    from_epoch,
                    previous_hash,
                    paused,
                    audit_hash,
                    at_ms,
                ),
            ),
            (
                (QUERIES / "state_postcondition.sql").read_text(encoding="utf-8"),
                (
                    postcondition_id,
                    tenant,
                    "metadata",
                    phase,
                    writer,
                    paused,
                    from_epoch + 1,
                    audit_hash,
                    at_ms,
                    phase_epoch,
                ),
            ),
            (
                (QUERIES / "assertion_cleanup.sql").read_text(encoding="utf-8"),
                (gate_id,),
            ),
            (
                (QUERIES / "assertion_cleanup.sql").read_text(encoding="utf-8"),
                (postcondition_id,),
            ),
        ],
    )


def main() -> int:
    database = apply_migrations()
    tenant = "00000000-0000-0000-0000-000000000017"
    other = "00000000-0000-0000-0000-000000000018"
    seed_scope(database, tenant, 0)
    seed_scope(database, other, 0)
    transition(
        database,
        tenant,
        from_phase="legacy_authoritative",
        to_phase="shadow_read",
        from_epoch=0,
        previous_hash=ZERO,
        audit_hash="1" * 64,
        at_ms=100,
    )
    seed_policy(database, tenant, 100)
    seed_policy(database, other, 0)
    transition(
        database,
        tenant,
        from_phase="shadow_read",
        to_phase="dual_write",
        from_epoch=1,
        previous_hash="1" * 64,
        audit_hash="2" * 64,
        at_ms=120,
    )
    database.execute(
        """INSERT INTO cutover_maintenance_windows(
             tenant_id, domain, starts_at_ms, ends_at_ms, approved_by_digest
           ) VALUES (?, 'metadata', 150, 250, ?)""",
        (tenant, "c" * 64),
    )
    observation_sql = (QUERIES / "shadow_observation_insert.sql").read_text(
        encoding="utf-8"
    )
    database.execute(
        observation_sql,
        (
            "3" * 64,
            tenant,
            "metadata",
            2,
            "video_list",
            "d" * 64,
            "e" * 64,
            "e" * 64,
            "match",
            130,
        ),
    )
    database.execute(
        observation_sql,
        (
            "4" * 64,
            tenant,
            "metadata",
            2,
            "video_list",
            "d" * 64,
            "f" * 64,
            "f" * 64,
            "match",
            180,
        ),
    )
    database.execute(
        observation_sql,
        (
            "4" * 64,
            tenant,
            "metadata",
            2,
            "video_list",
            "d" * 64,
            "f" * 64,
            "f" * 64,
            "match",
            180,
        ),
    )
    expect_integrity_error(
        database.execute,
        observation_sql,
        (
            "4" * 64,
            tenant,
            "metadata",
            2,
            "video_list",
            "d" * 64,
            "f" * 64,
            "e" * 64,
            "semantic_mismatch",
            180,
        ),
    )
    expect_integrity_error(
        database.execute,
        observation_sql,
        (
            "b" * 64,
            tenant,
            "metadata",
            1,
            "video_list",
            "d" * 64,
            "f" * 64,
            "f" * 64,
            "match",
            190,
        ),
    )
    signal_sql = (QUERIES / "signal_insert.sql").read_text(encoding="utf-8")
    database.execute(signal_sql, (tenant, "metadata", 2, "authority_contention", 130))
    database.execute(signal_sql, (tenant, "metadata", 2, "authority_contention", 190))
    expect_integrity_error(
        database.execute,
        signal_sql,
        (tenant, "metadata", 1, "authority_contention", 190),
    )
    database.execute(
        """INSERT INTO cutover_change_events(
             event_id, tenant_id, domain, sequence, authority_epoch,
             source_authority, event_digest, payload_ciphertext, state,
             occurred_at_ms, captured_at_ms
           ) VALUES ('event-1', ?, 'metadata', 1, 2, 'legacy', ?,
                     'ciphertext', 'pending', 170, 170)""",
        (tenant, "5" * 64),
    )

    snapshot_sql = (QUERIES / "authority_snapshot.sql").read_text(encoding="utf-8")
    rows = database.execute(snapshot_sql, (tenant, "metadata", 200)).fetchall()
    assert len(rows) == 1
    row = dict(rows[0])
    assert row["phase"] == "dual_write" and row["writer"] == "legacy"
    assert row["epoch"] == 2 and row["audit_head_consistent"] == 1
    assert row["singleton_phase"] == "legacy_authoritative"
    assert row["observation_window_started_at_ms"] == 150
    assert row["required_query_classes"] == row["covered_query_classes"] == 1
    assert row["shadow_observations"] == 1 and row["shadow_mismatches"] == 0
    assert row["pending_events"] == 1 and row["pending_lag_ms"] == 30
    assert row["authority_contention_events"] == 1
    assert row["signal_rollup_consistent"] == 1
    assert row["maintenance_window_active"] == 1
    other_row = dict(database.execute(snapshot_sql, (other, "metadata", 200)).fetchone())
    assert other_row["epoch"] == 0
    assert other_row["covered_query_classes"] == 0
    assert other_row["authority_contention_events"] == 0

    database.execute(
        """CREATE TABLE cutover_conformance_writes(
             tenant_id TEXT PRIMARY KEY, payload TEXT NOT NULL)"""
    )
    database.commit()
    database.execute("BEGIN")
    assertion(database, "success", tenant, "legacy", 2, 200)
    database.execute(
        "INSERT INTO cutover_conformance_writes VALUES (?, 'ok')", (tenant,)
    )
    database.execute(
        (QUERIES / "assertion_cleanup.sql").read_text(encoding="utf-8"),
        ("success",),
    )
    database.commit()
    assert database.execute(
        "SELECT payload FROM cutover_conformance_writes WHERE tenant_id = ?", (tenant,)
    ).fetchone()[0] == "ok"
    assert database.execute("SELECT COUNT(*) FROM cutover_repository_assertions_v1").fetchone()[0] == 0

    for name, scoped_tenant, scoped_writer, epoch, at_ms in (
        ("wrong-writer", tenant, "d1", 2, 200),
        ("wrong-epoch", tenant, "legacy", 1, 200),
        ("wrong-tenant", other, "legacy", 2, 200),
        ("mirror-disabled", other, "legacy", 0, 200),
        ("stale-time", tenant, "legacy", 2, 119),
    ):
        expect_integrity_error(
            assertion, database, name, scoped_tenant, scoped_writer, epoch, at_ms
        )

    database.rollback()

    database.execute(
        """UPDATE authority_state
           SET phase = 'd1_authoritative', authority = 'd1', epoch = 1,
               updated_at_ms = 200
           WHERE singleton = 1"""
    )
    database.commit()
    expect_integrity_error(assertion, database, "singleton-conflict", tenant, "legacy", 2, 200)
    database.rollback()
    database.execute(
        """UPDATE authority_state
           SET phase = 'rolled_back', authority = 'legacy', epoch = 2,
               updated_at_ms = 201
           WHERE singleton = 1"""
    )
    database.commit()

    expect_integrity_error(
        authority_transition_batch,
        database,
        tenant,
        from_phase="dual_write",
        to_phase="d1_authoritative",
        from_epoch=2,
        previous_hash="2" * 64,
        audit_hash="6" * 64,
        at_ms=200,
        require_health=True,
        require_drained=True,
    )
    state = database.execute(
        "SELECT phase, epoch, audit_head FROM cutover_authority_scopes "
        "WHERE tenant_id = ? AND domain = 'metadata'",
        (tenant,),
    ).fetchone()
    assert tuple(state) == ("dual_write", 2, "2" * 64)
    assert database.execute(
        "SELECT COUNT(*) FROM cutover_authority_audit WHERE tenant_id = ?",
        (tenant,),
    ).fetchone()[0] == 2

    database.execute(
        """UPDATE cutover_change_events
           SET state = 'applied', applied_at_ms = 205
           WHERE tenant_id = ? AND domain = 'metadata' AND event_id = 'event-1'""",
        (tenant,),
    )
    database.commit()
    authority_transition_batch(
        database,
        tenant,
        from_phase="dual_write",
        to_phase="d1_authoritative",
        from_epoch=2,
        previous_hash="2" * 64,
        audit_hash="6" * 64,
        at_ms=210,
        require_health=True,
        require_drained=True,
    )
    state = database.execute(
        "SELECT phase, writer, replay_paused, epoch, audit_head "
        "FROM cutover_authority_scopes WHERE tenant_id = ? AND domain = 'metadata'",
        (tenant,),
    ).fetchone()
    assert tuple(state) == ("d1_authoritative", "d1", 0, 3, "6" * 64)
    assert database.execute(
        "SELECT COUNT(*) FROM cutover_repository_assertions_v1"
    ).fetchone()[0] == 0

    expect_integrity_error(
        authority_transition_batch,
        database,
        tenant,
        from_phase="dual_write",
        to_phase="d1_authoritative",
        from_epoch=2,
        previous_hash="2" * 64,
        audit_hash="7" * 64,
        at_ms=215,
        require_health=True,
        require_drained=True,
    )
    assert database.execute(
        "SELECT COUNT(*) FROM cutover_authority_audit WHERE tenant_id = ?",
        (tenant,),
    ).fetchone()[0] == 3

    replay_control_batch(
        database,
        tenant,
        phase="d1_authoritative",
        writer="d1",
        action="pause",
        from_epoch=3,
        previous_hash="6" * 64,
        audit_hash="8" * 64,
        at_ms=220,
        phase_epoch=3,
    )
    expect_integrity_error(
        replay_control_batch,
        database,
        tenant,
        phase="d1_authoritative",
        writer="d1",
        action="resume",
        from_epoch=4,
        previous_hash="8" * 64,
        audit_hash="9" * 64,
        at_ms=260,
        phase_epoch=3,
    )
    state = database.execute(
        "SELECT replay_paused, epoch, audit_head FROM cutover_authority_scopes "
        "WHERE tenant_id = ? AND domain = 'metadata'",
        (tenant,),
    ).fetchone()
    assert tuple(state) == (1, 4, "8" * 64)
    replay_control_batch(
        database,
        tenant,
        phase="d1_authoritative",
        writer="d1",
        action="resume",
        from_epoch=4,
        previous_hash="8" * 64,
        audit_hash="a" * 64,
        at_ms=240,
        phase_epoch=3,
    )
    state = database.execute(
        "SELECT replay_paused, epoch, audit_head FROM cutover_authority_scopes "
        "WHERE tenant_id = ? AND domain = 'metadata'",
        (tenant,),
    ).fetchone()
    assert tuple(state) == (0, 5, "a" * 64)
    assert database.execute(
        "SELECT COUNT(*) FROM cutover_repository_assertions_v1"
    ).fetchone()[0] == 0

    print(
        json.dumps(
            {
                "append_only_health_signals": True,
                "atomic_audited_controls": True,
                "current_phase_freshness_window": True,
                "maintenance_window_enforced": True,
                "phase_epoch_freshness": True,
                "replay_drain_enforced": True,
                "shadow_digest_idempotency": True,
                "scoped_writer_epoch_fence": True,
                "signal_rollup_consistency": True,
                "singleton_compatibility_visible": True,
                "singleton_writer_conflict_fenced": True,
                "status": "ok",
                "tenant_domain_isolation": True,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, sqlite3.Error) as error:
        print(f"cutover authority conformance failed: {error}", file=sys.stderr)
        raise SystemExit(1) from None
