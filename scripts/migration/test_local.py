#!/usr/bin/env python3
"""Adversarial local proof for deterministic ETL and reversible cutover controls."""

from __future__ import annotations

import json
import pathlib
import shutil
import sqlite3
import sys
import tempfile
import threading
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import cutover
import etl


ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures" / "etl" / "v1"
AT = 1_735_689_700_000


def initialize_target(path: pathlib.Path) -> None:
    connection = sqlite3.connect(path)
    connection.executescript((FIXTURE / "target.sql").read_text(encoding="utf-8"))
    connection.close()


def bundle_files(bundle: pathlib.Path) -> dict[str, bytes]:
    return {
        path.relative_to(bundle).as_posix(): path.read_bytes()
        for path in sorted(bundle.rglob("*"))
        if path.is_file()
    }


def write_json(path: pathlib.Path, value: object) -> None:
    path.write_text(f"{etl.canonical(value)}\n", encoding="utf-8")


def assert_redacted(value: object) -> None:
    encoded = etl.canonical(value).casefold()
    for forbidden in (
        "@",
        "account-a-owner",
        "account-a-member",
        "private.value",
        "should.not.leak",
        "tenant-fixture-a",
        "tenant-fixture-b",
        "tenant-fixture-window",
        "injected target outage with private details",
    ):
        assert forbidden not in encoded, f"redacted evidence leaked {forbidden}"


def expect_error(callable_object, exception_type, *args, **kwargs) -> None:
    try:
        callable_object(*args, **kwargs)
    except exception_type:
        return
    raise AssertionError(f"expected {exception_type.__name__}")


def test_etl(root: pathlib.Path) -> tuple[pathlib.Path, pathlib.Path, dict[str, object]]:
    plan = etl.load_plan(FIXTURE / "plan.json")
    boolean_column = next(
        column for table in plan.tables for column in table.columns if column.transform == "boolean"
    )
    integer_column = next(
        column for table in plan.tables for column in table.columns if column.transform == "wire_integer"
    )
    expect_error(etl.transform_value, ValueError, boolean_column, 1.0)
    expect_error(etl.transform_value, ValueError, boolean_column, 0.0)
    expect_error(etl.transform_value, ValueError, integer_column, True)
    expect_error(etl.transform_value, ValueError, integer_column, False)
    bundle = root / "bundle"
    duplicate_bundle = root / "bundle-duplicate"
    manifest = etl.export_bundle(FIXTURE / "source.ndjson", plan, bundle, chunk_rows=1)
    duplicate = etl.export_bundle(
        FIXTURE / "source.ndjson", plan, duplicate_bundle, chunk_rows=1
    )
    assert manifest == duplicate
    assert bundle_files(bundle) == bundle_files(duplicate_bundle)
    assert manifest["row_count"] == 10
    assert manifest["reject_count"] == 0
    assert (bundle.stat().st_mode & 0o077) == 0
    assert all((path.stat().st_mode & 0o077) == 0 for path in bundle.rglob("*") if path.is_file())
    expect_error(
        etl.export_bundle,
        etl.EtlError,
        FIXTURE / "source.ndjson",
        plan,
        bundle,
        1,
    )

    target = root / "target.sqlite"
    initialize_target(target)
    dry = etl.import_bundle(
        target, bundle, dry_run=True, max_rows_per_second=0
    )
    assert dry == {"applied_chunks": 10, "skipped_chunks": 0, "validated_rows": 10}
    connection = sqlite3.connect(target)
    assert connection.execute("SELECT COUNT(*) FROM accounts").fetchone()[0] == 0
    assert connection.execute(
        "SELECT COUNT(*) FROM sqlite_master WHERE name = '_frame_etl_checkpoints'"
    ).fetchone()[0] == 0
    connection.close()

    expect_error(
        etl.import_bundle,
        etl.InjectedInterruption,
        target,
        bundle,
        dry_run=False,
        max_rows_per_second=0,
        interrupt_after_chunks=3,
    )
    resumed = etl.import_bundle(
        target, bundle, dry_run=False, max_rows_per_second=0
    )
    assert resumed == {"applied_chunks": 7, "skipped_chunks": 3, "validated_rows": 7}
    replayed = etl.import_bundle(
        target, bundle, dry_run=False, max_rows_per_second=0
    )
    assert replayed == {"applied_chunks": 0, "skipped_chunks": 10, "validated_rows": 0}

    report = dict(etl.reconcile_bundle(target, bundle))
    assert report["clean"] is True
    assert report["unexplained_mismatches"] == 0
    assert report["relationships"] == {
        "source_violations": 0,
        "target_violations": 0,
        "target_engine_violations": 0,
    }
    assert_redacted(report)

    connection = sqlite3.connect(target)
    original_budget = connection.execute(
        "SELECT budget_micros FROM projects WHERE id = 'project-a'"
    ).fetchone()[0]
    connection.execute("UPDATE projects SET budget_micros = 999 WHERE id = 'project-a'")
    connection.commit()
    connection.close()
    mismatch = dict(etl.reconcile_bundle(target, bundle))
    assert mismatch["clean"] is False
    assert mismatch["unexplained_mismatches"] >= 2
    assert_redacted(mismatch)
    connection = sqlite3.connect(target)
    connection.execute(
        "UPDATE projects SET budget_micros = ? WHERE id = 'project-a'", (original_budget,)
    )
    connection.commit()
    connection.close()

    malformed = root / "malformed-bundle"
    malformed_manifest = etl.export_bundle(
        FIXTURE / "source-malformed.ndjson", plan, malformed, chunk_rows=2
    )
    assert malformed_manifest["reject_count"] == 1
    quarantine = (malformed / "quarantine.ndjson").read_text(encoding="utf-8")
    assert "invalid_boolean" in quarantine
    assert "@" not in quarantine
    assert "account-invalid" not in quarantine
    blocked = dict(etl.reconcile_bundle(target, malformed))
    assert blocked["clean"] is False
    assert blocked["blocked_by_quarantine"] is True
    assert_redacted(blocked)

    tampered = root / "tampered-bundle"
    shutil.copytree(bundle, tampered)
    tampered_manifest = etl.load_manifest(tampered)
    chunk = tampered_manifest["tables"][0]["tenants"][0]["chunks"][0]
    first_chunk = tampered / pathlib.PurePosixPath(chunk["path"])
    first_chunk.write_bytes(first_chunk.read_bytes() + b"{}\n")
    expect_error(etl.read_chunk, etl.EtlError, tampered, chunk)
    return target, bundle, report


def test_cutover(
    root: pathlib.Path, target: pathlib.Path
) -> tuple[dict[str, object], int]:
    config = cutover.load_config(FIXTURE / "plan.json")
    state = root / "cutover" / "state.sqlite"
    operator = root / "operator.credential"
    operator.write_text("local-rehearsal-operator-v1\n", encoding="utf-8")
    operator.chmod(0o600)

    expect_error(
        cutover.initialize_scope,
        cutover.CutoverError,
        state,
        config,
        "tenant-not-approved",
        "metadata",
        AT,
    )

    initialized = cutover.initialize_scope(
        state, config, "tenant-fixture-a", "metadata", AT
    )
    assert initialized["writer"] == "legacy" and initialized["epoch"] == 0
    expect_error(
        cutover.transition,
        cutover.CutoverError,
        state,
        config,
        "tenant-fixture-a",
        "metadata",
        "d1_authoritative",
        0,
        operator,
        FIXTURE / "evidence-d1.json",
        AT + 1,
    )
    shadow = cutover.transition(
        state,
        config,
        "tenant-fixture-a",
        "metadata",
        "shadow_read",
        0,
        operator,
        FIXTURE / "evidence-shadow.json",
        AT + 1,
    )
    assert shadow["epoch"] == 1 and shadow["writer"] == "legacy"
    normalized = dict(
        cutover.compare_shadow(
            state,
            config,
            "metadata",
            FIXTURE / "shadow-normalized-match.json",
            AT + 2,
        )
    )
    assert normalized["classification"] == "match"
    assert normalized["normalizations_applied"] is True
    assert_redacted(normalized)
    dual = cutover.transition(
        state,
        config,
        "tenant-fixture-a",
        "metadata",
        "dual_write",
        1,
        operator,
        FIXTURE / "evidence-dual.json",
        AT + 3,
    )
    assert dual["epoch"] == 2 and dual["writer"] == "legacy"

    # A stale compare-and-swap is rejected and leaves an operator-safe contention signal.
    expect_error(
        cutover.transition,
        cutover.CutoverError,
        state,
        config,
        "tenant-fixture-a",
        "metadata",
        "d1_authoritative",
        1,
        operator,
        FIXTURE / "evidence-d1.json",
        AT + 4,
    )
    contention_status = cutover.status(
        state, config, "tenant-fixture-a", "metadata", AT + 4
    )
    assert contention_status["alerts"]["authority_contention"]["active"] is True

    paused = cutover.replay_control(
        state,
        config,
        "tenant-fixture-a",
        "metadata",
        "pause",
        2,
        operator,
        AT + 5,
    )
    assert paused["epoch"] == 3
    expect_error(
        cutover.replay_events,
        cutover.CutoverError,
        state,
        target,
        config,
        "tenant-fixture-a",
        "metadata",
        AT + 6,
        10,
    )
    resumed = cutover.replay_control(
        state,
        config,
        "tenant-fixture-a",
        "metadata",
        "resume",
        3,
        operator,
        AT + 7,
    )
    assert resumed["epoch"] == 4
    captured = cutover.capture_events(
        state, config, "metadata", FIXTURE / "events.ndjson", AT + 8
    )
    assert captured == {"captured": 2, "idempotent_duplicates": 0}
    duplicates = cutover.capture_events(
        state, config, "metadata", FIXTURE / "events.ndjson", AT + 9
    )
    assert duplicates == {"captured": 0, "idempotent_duplicates": 2}
    wrong_writer_path = root / "wrong-writer-event.ndjson"
    wrong_writer_event = json.loads(
        (FIXTURE / "event-poison.ndjson").read_text(encoding="utf-8")
    )
    wrong_writer_event["source_authority"] = "d1"
    write_json(wrong_writer_path, wrong_writer_event)
    expect_error(
        cutover.capture_events,
        cutover.CutoverError,
        state,
        config,
        "metadata",
        wrong_writer_path,
        AT + 10,
    )
    poison = cutover.capture_events(
        state, config, "metadata", FIXTURE / "event-poison.ndjson", AT + 10
    )
    assert poison["captured"] == 1

    # Durable envelope fields and payload digest are revalidated before target I/O.
    state_db = cutover.open_state(state)
    original_payload = state_db.execute(
        """SELECT payload_json FROM captured_events
           WHERE tenant_digest = ? AND domain = 'metadata' AND sequence = 1""",
        (etl.tenant_digest("tenant-fixture-a"),),
    ).fetchone()["payload_json"]
    tampered_payload = json.loads(original_payload)
    tampered_payload["operation"]["source_row"]["budget"] = "999.000000"
    state_db.execute(
        """UPDATE captured_events SET payload_json = ?
           WHERE tenant_digest = ? AND domain = 'metadata' AND sequence = 1""",
        (etl.canonical(tampered_payload), etl.tenant_digest("tenant-fixture-a")),
    )
    state_db.commit()
    state_db.close()
    expect_error(
        cutover.replay_events,
        cutover.CutoverError,
        state,
        target,
        config,
        "tenant-fixture-a",
        "metadata",
        AT + 10,
        1,
    )
    state_db = cutover.open_state(state)
    state_db.execute(
        """UPDATE captured_events SET payload_json = ?
           WHERE tenant_digest = ? AND domain = 'metadata' AND sequence = 1""",
        (original_payload, etl.tenant_digest("tenant-fixture-a")),
    )
    state_db.commit()
    state_db.close()

    # A target outage before commit is observable but retryable: the source event
    # remains pending and is applied by the next healthy replay.
    original_apply = cutover._apply_event

    def unavailable_target(*_args, **_kwargs) -> None:
        raise sqlite3.OperationalError("injected target outage with private details")

    cutover._apply_event = unavailable_target
    try:
        expect_error(
            cutover.replay_events,
            cutover.CutoverError,
            state,
            target,
            config,
            "tenant-fixture-a",
            "metadata",
            AT + 11,
            1,
        )
    finally:
        cutover._apply_event = original_apply
    lagging_status = cutover.status(
        state, config, "tenant-fixture-a", "metadata", AT + 11
    )
    assert lagging_status["pending_events"] == 3
    assert lagging_status["alerts"]["replay_lag"]["active"] is True
    assert lagging_status["alerts"]["replay_write_failure"]["active"] is True
    assert_redacted(lagging_status)

    # A pause races an in-flight target commit. The replay transaction holds the
    # authority fence through acknowledgement, so pause cannot return early.
    apply_entered = threading.Event()
    release_apply = threading.Event()
    pause_started = threading.Event()
    pause_finished = threading.Event()
    replay_result: dict[str, object] = {}
    pause_result: dict[str, object] = {}

    def blocking_apply(*args, **kwargs) -> None:
        apply_entered.set()
        assert release_apply.wait(5), "timed out releasing injected replay boundary"
        original_apply(*args, **kwargs)

    def run_replay() -> None:
        try:
            replay_result["value"] = cutover.replay_events(
                state,
                target,
                config,
                "tenant-fixture-a",
                "metadata",
                AT + 12,
                1,
            )
        except BaseException as error:  # surfaced in the parent thread below
            replay_result["error"] = error

    def run_pause() -> None:
        pause_started.set()
        try:
            pause_result["value"] = cutover.replay_control(
                state,
                config,
                "tenant-fixture-a",
                "metadata",
                "pause",
                4,
                operator,
                AT + 13,
            )
        except BaseException as error:  # surfaced in the parent thread below
            pause_result["error"] = error
        finally:
            pause_finished.set()

    cutover._apply_event = blocking_apply
    replay_thread = threading.Thread(target=run_replay, name="cutover-replay-race")
    pause_thread = threading.Thread(target=run_pause, name="cutover-pause-race")
    try:
        replay_thread.start()
        assert apply_entered.wait(5), "replay never reached the injected boundary"
        pause_thread.start()
        assert pause_started.wait(5)
        assert not pause_finished.wait(0.05), "pause bypassed an in-flight replay fence"
        release_apply.set()
        replay_thread.join(5)
        pause_thread.join(5)
        assert not replay_thread.is_alive() and not pause_thread.is_alive()
    finally:
        release_apply.set()
        cutover._apply_event = original_apply
    assert "error" not in replay_result, replay_result.get("error")
    assert "error" not in pause_result, pause_result.get("error")
    assert replay_result["value"] == {
        "applied": 1,
        "recovered_after_commit": 0,
        "dead_lettered": 0,
    }
    assert pause_result["value"]["epoch"] == 5
    resumed_after_race = cutover.replay_control(
        state,
        config,
        "tenant-fixture-a",
        "metadata",
        "resume",
        5,
        operator,
        AT + 14,
    )
    assert resumed_after_race["epoch"] == 6
    gap_event_path = root / "gap-event.ndjson"
    gap_event = json.loads((FIXTURE / "events.ndjson").read_text(encoding="utf-8").splitlines()[0])
    gap_event["event_id"] = "fixture-event-gap-0005"
    gap_event["sequence"] = 5
    gap_event["authority_epoch"] = 6
    gap_event["occurred_at_ms"] = AT + 14
    write_json(gap_event_path, gap_event)
    expect_error(
        cutover.capture_events,
        cutover.CutoverError,
        state,
        config,
        "metadata",
        gap_event_path,
        AT + 14,
    )

    expect_error(
        cutover.replay_events,
        cutover.InjectedReplayInterruption,
        state,
        target,
        config,
        "tenant-fixture-a",
        "metadata",
        AT + 15,
        10,
        True,
    )
    replayed = cutover.replay_events(
        state,
        target,
        config,
        "tenant-fixture-a",
        "metadata",
        AT + 16,
        10,
    )
    assert replayed == {"applied": 0, "recovered_after_commit": 1, "dead_lettered": 1}
    connection = sqlite3.connect(target)
    assert connection.execute(
        "SELECT COUNT(*) FROM projects WHERE id = 'project-a-catchup'"
    ).fetchone()[0] == 1
    assert connection.execute(
        "SELECT quota_micros FROM accounts WHERE id = 'account-a-member'"
    ).fetchone()[0] == 2_000_000
    connection.close()

    mismatch = dict(
        cutover.compare_shadow(
            state,
            config,
            "metadata",
            FIXTURE / "shadow-semantic-mismatch.json",
            AT + 17,
        )
    )
    assert mismatch["classification"] == "semantic_mismatch"
    assert_redacted(mismatch)
    expect_error(
        cutover.transition,
        cutover.CutoverError,
        state,
        config,
        "tenant-fixture-a",
        "metadata",
        "d1_authoritative",
        6,
        operator,
        FIXTURE / "evidence-d1.json",
        AT + 18,
    )
    post_control_duplicates = cutover.capture_events(
        state, config, "metadata", FIXTURE / "events.ndjson", AT + 19
    )
    assert post_control_duplicates == {"captured": 0, "idempotent_duplicates": 2}

    status = dict(
        cutover.status(state, config, "tenant-fixture-a", "metadata", AT + 20_000)
    )
    assert status["writer"] == "legacy"
    assert status["applied_events"] == 2
    assert status["dead_letter_events"] == 1
    assert status["pending_events"] == 0
    assert status["latest_shadow_window"]["coverage_complete"] is True
    assert status["latest_shadow_window"]["mismatches"] == 1
    assert status["recent_lost_acknowledgements"] == 1
    for alert in (
        "dead_letter",
        "replay_lost_ack",
        "rollback_readiness",
        "shadow_mismatch",
    ):
        assert status["alerts"][alert]["active"] is True, (alert, status["alerts"][alert])
    assert status["audit_chain_valid"] is True
    assert_redacted(status)
    expired_signal_window = cutover.status(
        state, config, "tenant-fixture-a", "metadata", AT + 80_000
    )["latest_operational_window"]
    assert expired_signal_window["authority_contention"] == 0
    assert expired_signal_window["replay_write_failure"] == 0
    assert expired_signal_window["replay_lost_ack"] == 0

    legacy_fence = dict(
        cutover.verify_writer_fence(
            state, config, "tenant-fixture-a", "metadata", "legacy", 6
        )
    )
    assert legacy_fence["authorized"] is True
    expect_error(
        cutover.verify_writer_fence,
        cutover.CutoverError,
        state,
        config,
        "tenant-fixture-a",
        "metadata",
        "d1",
        6,
    )
    assert_redacted(legacy_fence)

    # A clean second scope proves fenced cutover and rollback while retaining one writer.
    rehearsal_started_ns = time.monotonic_ns()
    cutover.initialize_scope(state, config, "tenant-fixture-b", "metadata", AT + 30)
    cutover.transition(
        state,
        config,
        "tenant-fixture-b",
        "metadata",
        "shadow_read",
        0,
        operator,
        FIXTURE / "evidence-shadow.json",
        AT + 31,
    )
    shadow_b_path = root / "shadow-b.json"
    shadow_b = json.loads(
        (FIXTURE / "shadow-normalized-match.json").read_text(encoding="utf-8")
    )
    shadow_b["tenant"] = "tenant-fixture-b"
    write_json(shadow_b_path, shadow_b)
    compared_b = cutover.compare_shadow(
        state, config, "metadata", shadow_b_path, AT + 32
    )
    assert compared_b["classification"] == "match"
    assert_redacted(compared_b)
    cutover.transition(
        state,
        config,
        "tenant-fixture-b",
        "metadata",
        "dual_write",
        1,
        operator,
        FIXTURE / "evidence-dual.json",
        AT + 33,
    )
    expect_error(
        cutover.transition,
        cutover.CutoverError,
        state,
        config,
        "tenant-fixture-b",
        "metadata",
        "d1_authoritative",
        2,
        operator,
        FIXTURE / "evidence-d1.json",
        AT + 34,
    )
    # Promotion cannot reuse a clean observation collected in the prior phase.
    compared_dual_b = cutover.compare_shadow(
        state, config, "metadata", shadow_b_path, AT + 34
    )
    assert compared_dual_b["classification"] == "match"
    d1 = cutover.transition(
        state,
        config,
        "tenant-fixture-b",
        "metadata",
        "d1_authoritative",
        2,
        operator,
        FIXTURE / "evidence-d1.json",
        AT + 34,
    )
    assert d1["writer"] == "d1" and d1["epoch"] == 3
    d1_fence = cutover.verify_writer_fence(
        state, config, "tenant-fixture-b", "metadata", "d1", 3
    )
    assert d1_fence["authorized"] is True

    # During the reversible D1 canary, D1 remains the sole writer and emits an
    # ordered mirror event. A local legacy projection catches up before rollback.
    canary_event_path = root / "d1-canary-event.ndjson"
    write_json(
        canary_event_path,
        {
            "event_id": "fixture-event-0001",
            "tenant": "tenant-fixture-b",
            "domain": "metadata",
            "sequence": 1,
            "authority_epoch": 3,
            "source_authority": "d1",
            "occurred_at_ms": AT + 34,
            "operation": {
                "kind": "upsert",
                "table": "accounts",
                "source_row": {
                    "id": "account-b-canary",
                    "tenant": "tenant-fixture-b",
                    "email": "canary.b@example.invalid",
                    "enabled": "1",
                    "quota": "3.000000",
                    "profile": "{}",
                    "created_at": "2025-01-01T00:01:04Z",
                    "legacy_tier": "pro_legacy",
                    "deleted_at": None,
                },
            },
        },
    )
    assert cutover.capture_events(
        state, config, "metadata", canary_event_path, AT + 35
    ) == {"captured": 1, "idempotent_duplicates": 0}
    legacy_projection = root / "legacy-projection.sqlite"
    initialize_target(legacy_projection)
    assert cutover.replay_events(
        state,
        legacy_projection,
        config,
        "tenant-fixture-b",
        "metadata",
        AT + 36,
        10,
    ) == {"applied": 1, "recovered_after_commit": 0, "dead_lettered": 0}
    legacy_projection_db = sqlite3.connect(legacy_projection)
    assert legacy_projection_db.execute(
        "SELECT quota_micros FROM accounts WHERE id = 'account-b-canary'"
    ).fetchone()[0] == 3_000_000
    legacy_projection_db.execute(
        """INSERT INTO accounts(
             id, tenant_id, email_normalized, enabled, quota_micros, profile_json,
             created_at_ms, tier, deleted_at_ms
           ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)""",
        (
            "account-b-delete",
            "tenant-fixture-b",
            "delete.b@example.invalid",
            1,
            1_000_000,
            "{}",
            AT,
            "free",
            None,
        ),
    )
    legacy_projection_db.commit()
    legacy_projection_db.close()
    delete_event_path = root / "d1-canary-delete.ndjson"
    write_json(
        delete_event_path,
        {
            "event_id": "fixture-event-b-canary-0002",
            "tenant": "tenant-fixture-b",
            "domain": "metadata",
            "sequence": 2,
            "authority_epoch": 3,
            "source_authority": "d1",
            "occurred_at_ms": AT + 36,
            "operation": {
                "kind": "delete",
                "table": "accounts",
                "source_key": {
                    "id": "account-b-delete",
                    "tenant": "tenant-fixture-b",
                },
            },
        },
    )
    assert cutover.capture_events(
        state, config, "metadata", delete_event_path, AT + 36
    ) == {"captured": 1, "idempotent_duplicates": 0}
    assert cutover.replay_events(
        state,
        legacy_projection,
        config,
        "tenant-fixture-b",
        "metadata",
        AT + 36,
        10,
    ) == {"applied": 1, "recovered_after_commit": 0, "dead_lettered": 0}
    legacy_projection_db = sqlite3.connect(legacy_projection)
    assert legacy_projection_db.execute(
        "SELECT COUNT(*) FROM accounts WHERE id = 'account-b-delete'"
    ).fetchone()[0] == 0
    legacy_projection_db.close()
    rolled_back = cutover.transition(
        state,
        config,
        "tenant-fixture-b",
        "metadata",
        "rolled_back",
        3,
        operator,
        FIXTURE / "evidence-rollback.json",
        AT + 37,
    )
    assert rolled_back["writer"] == "legacy" and rolled_back["epoch"] == 4
    expect_error(
        cutover.verify_writer_fence,
        cutover.CutoverError,
        state,
        config,
        "tenant-fixture-b",
        "metadata",
        "d1",
        3,
    )
    assert cutover.verify_writer_fence(
        state, config, "tenant-fixture-b", "metadata", "legacy", 4
    )["authorized"] is True
    rehearsal_elapsed_ms = max(
        1, (time.monotonic_ns() - rehearsal_started_ns) // 1_000_000
    )

    # Forward controls are confined to configured windows; emergency rollback is not.
    cutover.initialize_scope(
        state, config, "tenant-fixture-window", "metadata", config.maintenance_windows[-1][1]
    )
    expect_error(
        cutover.transition,
        cutover.CutoverError,
        state,
        config,
        "tenant-fixture-window",
        "metadata",
        "shadow_read",
        0,
        operator,
        FIXTURE / "evidence-shadow.json",
        config.maintenance_windows[-1][1] + 1,
    )

    # Credentials are non-followed, owner-private controls and all JSON controls are bounded.
    insecure_operator = root / "insecure.credential"
    insecure_operator.write_text("local-rehearsal-operator-v1\n", encoding="utf-8")
    insecure_operator.chmod(0o644)
    operator_link = root / "operator-link.credential"
    operator_link.symlink_to(operator)
    for unsafe_operator in (insecure_operator, operator_link):
        expect_error(
            cutover.transition,
            cutover.CutoverError,
            state,
            config,
            "tenant-fixture-b",
            "metadata",
            "dual_write",
            4,
            unsafe_operator,
            FIXTURE / "evidence-dual.json",
            AT + 38,
        )
    original_control_limit = cutover.MAX_CONTROL_FILE_BYTES
    cutover.MAX_CONTROL_FILE_BYTES = 1
    try:
        expect_error(
            cutover.transition,
            cutover.CutoverError,
            state,
            config,
            "tenant-fixture-b",
            "metadata",
            "dual_write",
            4,
            operator,
            FIXTURE / "evidence-dual.json",
            AT + 38,
        )
    finally:
        cutover.MAX_CONTROL_FILE_BYTES = original_control_limit

    audit_a = dict(
        cutover.verify_audit_chain(state, config, "tenant-fixture-a", "metadata")
    )
    audit_b = dict(
        cutover.verify_audit_chain(state, config, "tenant-fixture-b", "metadata")
    )
    assert audit_a["valid"] is True and audit_a["epoch"] == 6
    assert audit_b["valid"] is True and audit_b["epoch"] == 4
    assert_redacted(audit_a)

    state_db = cutover.open_state(state)
    audit_rows = state_db.execute(
        "SELECT * FROM authority_audit ORDER BY tenant_digest, to_epoch"
    ).fetchall()
    for row in audit_rows:
        assert row["to_epoch"] == row["from_epoch"] + 1
    for tenant_hash in {row["tenant_digest"] for row in audit_rows}:
        scoped = [row for row in audit_rows if row["tenant_digest"] == tenant_hash]
        previous = "0" * 64
        for row in scoped:
            assert row["previous_hash"] == previous
            previous = row["audit_hash"]
    expect_error(
        state_db.execute,
        sqlite3.DatabaseError,
        "UPDATE authority_audit SET evidence_digest = ? WHERE audit_hash = ?",
        ("0" * 64, audit_rows[0]["audit_hash"]),
    )
    state_db.rollback()
    state_db.close()

    tampered_state = root / "cutover" / "tampered.sqlite"
    source_db = sqlite3.connect(state)
    copied_db = sqlite3.connect(tampered_state)
    source_db.backup(copied_db)
    copied_db.close()
    source_db.close()
    tampered_state.chmod(0o600)
    tamper = sqlite3.connect(tampered_state)
    tamper.execute("DROP TRIGGER authority_audit_immutable_update")
    tamper.execute(
        "UPDATE authority_audit SET evidence_digest = ? WHERE audit_hash = (SELECT audit_hash FROM authority_audit ORDER BY to_epoch LIMIT 1)",
        ("0" * 64,),
    )
    tamper.commit()
    tamper.close()
    expect_error(
        cutover.verify_audit_chain,
        cutover.CutoverError,
        tampered_state,
        config,
        "tenant-fixture-a",
        "metadata",
    )

    legacy_state_directory = root / "legacy-cutover-state"
    legacy_state_directory.mkdir(mode=0o700)
    legacy_state_path = legacy_state_directory / "state.sqlite"
    legacy_state = sqlite3.connect(legacy_state_path)
    legacy_state.execute(
        """CREATE TABLE authority_state (
             tenant_digest TEXT NOT NULL,
             domain TEXT NOT NULL,
             phase TEXT NOT NULL,
             writer TEXT NOT NULL,
             mirror_enabled INTEGER NOT NULL,
             replay_paused INTEGER NOT NULL,
             epoch INTEGER NOT NULL,
             audit_head TEXT NOT NULL,
             updated_at_ms INTEGER NOT NULL,
             PRIMARY KEY(tenant_digest, domain)
           )"""
    )
    legacy_state.commit()
    legacy_state.close()
    legacy_state_path.chmod(0o600)
    upgraded_state = cutover.open_state(legacy_state_path)
    upgraded_columns = {
        row["name"] for row in upgraded_state.execute("PRAGMA table_info(authority_state)")
    }
    upgraded_state.close()
    assert "rollback_ready" in upgraded_columns
    assert (state.stat().st_mode & 0o077) == 0
    return status, rehearsal_elapsed_ms


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-etl-reconciliation-") as directory:
        root = pathlib.Path(directory)
        target, _bundle, reconciliation = test_etl(root)
        cutover_status, cutover_rehearsal_elapsed_ms = test_cutover(root, target)
        evidence = {
            "deterministic_bundle": True,
            "dry_run_no_target_mutation": True,
            "interruption_resume": True,
            "idempotent_replay": True,
            "quarantine_redaction": True,
            "row_relationship_aggregate_semantic_reconciliation": reconciliation["clean"],
            "shadow_normalization_and_seeded_divergence": True,
            "latest_window_shadow_query_coverage": True,
            "phase_fresh_shadow_query_coverage": True,
            "exact_operational_signal_windows": True,
            "explicit_cutover_slo_alerts": True,
            "seeded_mismatch_dashboard_contract": True,
            "retryable_target_outage_recovered": True,
            "replay_pause_race_serialized": True,
            "target_commit_recovery": True,
            "captured_event_envelope_tamper_rejected": True,
            "capture_source_writer_and_contiguous_sequence_fenced": True,
            "tenant_scoped_event_identity": True,
            "ordered_delete_replay": True,
            "d1_canary_change_preserved_before_rollback": True,
            "poison_event_dead_letter": cutover_status["dead_letter_events"] == 1,
            "audited_pause_resume_cutover_rollback": True,
            "audit_hash_chain_tamper_rejected": True,
            "per_tenant_domain_single_writer_fences": True,
            "bounded_owner_private_controls": True,
            "no_pii_or_secrets_in_cutover_reports": True,
            "local_tenant_domain_cutover_rollback_elapsed_ms": cutover_rehearsal_elapsed_ms,
            "production_evidence": False,
        }
        assert_redacted(evidence)
        print(etl.canonical(evidence))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
