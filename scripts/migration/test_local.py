#!/usr/bin/env python3
"""Adversarial local proof for deterministic ETL and reversible cutover controls."""

from __future__ import annotations

import json
import pathlib
import shutil
import sqlite3
import sys
import tempfile

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


def test_cutover(root: pathlib.Path, target: pathlib.Path) -> dict[str, object]:
    config = cutover.load_config(FIXTURE / "plan.json")
    state = root / "cutover" / "state.sqlite"
    operator = root / "operator.credential"
    operator.write_text("local-rehearsal-operator-v1\n", encoding="utf-8")
    operator.chmod(0o600)

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

    paused = cutover.replay_control(
        state,
        config,
        "tenant-fixture-a",
        "metadata",
        "pause",
        2,
        operator,
        AT + 4,
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
        AT + 5,
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
        AT + 6,
    )
    assert resumed["epoch"] == 4
    captured = cutover.capture_events(
        state, config, "metadata", FIXTURE / "events.ndjson", AT + 7
    )
    assert captured == {"captured": 2, "idempotent_duplicates": 0}
    duplicates = cutover.capture_events(
        state, config, "metadata", FIXTURE / "events.ndjson", AT + 8
    )
    assert duplicates == {"captured": 0, "idempotent_duplicates": 2}
    poison = cutover.capture_events(
        state, config, "metadata", FIXTURE / "event-poison.ndjson", AT + 9
    )
    assert poison["captured"] == 1
    expect_error(
        cutover.replay_events,
        cutover.InjectedReplayInterruption,
        state,
        target,
        config,
        "tenant-fixture-a",
        "metadata",
        AT + 10,
        10,
        True,
    )
    replayed = cutover.replay_events(
        state,
        target,
        config,
        "tenant-fixture-a",
        "metadata",
        AT + 11,
        10,
    )
    assert replayed == {"applied": 1, "recovered_after_commit": 1, "dead_lettered": 1}
    status = dict(
        cutover.status(state, config, "tenant-fixture-a", "metadata", AT + 20_000)
    )
    assert status["writer"] == "legacy"
    assert status["applied_events"] == 2
    assert status["dead_letter_events"] == 1
    assert status["pending_events"] == 0
    assert_redacted(status)
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
            AT + 12,
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
        4,
        operator,
        FIXTURE / "evidence-d1.json",
        AT + 13,
    )

    # A clean second scope proves fenced cutover and rollback while retaining one writer.
    cutover.initialize_scope(state, config, "tenant-fixture-b", "metadata", AT + 20)
    cutover.transition(
        state,
        config,
        "tenant-fixture-b",
        "metadata",
        "shadow_read",
        0,
        operator,
        FIXTURE / "evidence-shadow.json",
        AT + 21,
    )
    cutover.transition(
        state,
        config,
        "tenant-fixture-b",
        "metadata",
        "dual_write",
        1,
        operator,
        FIXTURE / "evidence-dual.json",
        AT + 22,
    )
    d1 = cutover.transition(
        state,
        config,
        "tenant-fixture-b",
        "metadata",
        "d1_authoritative",
        2,
        operator,
        FIXTURE / "evidence-d1.json",
        AT + 23,
    )
    assert d1["writer"] == "d1" and d1["epoch"] == 3
    rolled_back = cutover.transition(
        state,
        config,
        "tenant-fixture-b",
        "metadata",
        "rolled_back",
        3,
        operator,
        FIXTURE / "evidence-rollback.json",
        AT + 24,
    )
    assert rolled_back["writer"] == "legacy" and rolled_back["epoch"] == 4

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
    state_db.close()
    assert (state.stat().st_mode & 0o077) == 0
    return status


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-etl-reconciliation-") as directory:
        root = pathlib.Path(directory)
        target, _bundle, reconciliation = test_etl(root)
        cutover_status = test_cutover(root, target)
        evidence = {
            "deterministic_bundle": True,
            "dry_run_no_target_mutation": True,
            "interruption_resume": True,
            "idempotent_replay": True,
            "quarantine_redaction": True,
            "row_relationship_aggregate_semantic_reconciliation": reconciliation["clean"],
            "shadow_normalization_and_seeded_divergence": True,
            "target_commit_recovery": True,
            "poison_event_dead_letter": cutover_status["dead_letter_events"] == 1,
            "audited_pause_resume_cutover_rollback": True,
            "production_evidence": False,
        }
        assert_redacted(evidence)
        print(etl.canonical(evidence))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
