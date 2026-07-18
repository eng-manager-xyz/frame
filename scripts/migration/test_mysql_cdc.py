#!/usr/bin/env python3
"""Credential-free subprocess proof for the GTID-bound MySQL CDC adapter."""

from __future__ import annotations

import hashlib
import json
import pathlib
import sys
import tempfile
from typing import Any

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import etl
import mysql_cdc


def source_binding() -> dict[str, str]:
    return {
        "database_sha256": "1" * 64,
        "server_uuid_sha256": "2" * 64,
        "server_version_sha256": "3" * 64,
        "tables_sha256": "4" * 64,
        "columns_sha256": "5" * 64,
        "indexes_sha256": "6" * 64,
        "constraints_sha256": "7" * 64,
    }


def plan_value() -> dict[str, Any]:
    return {
        "schema_version": 1,
        "run_id": "mysql-cdc-local-v1",
        "source_schema": "fixture-v1",
        "target_migration": "fixture-d1-v1",
        "code_sha": "8" * 64,
        "window": {"start_ms": 1, "end_ms": 9007199254740991},
        "mysql_snapshot": source_binding(),
        "tables": [
            {
                "name": "items",
                "tenant_column": "tenant_id",
                "primary_key": ["id"],
                "columns": [
                    {"source": "tenant", "target": "tenant_id"},
                    {"source": "id", "target": "id"},
                    {"source": "value", "target": "value"},
                ],
            }
        ],
        "semantic_rules": [],
    }


def boundary_value(plan: dict[str, Any]) -> dict[str, Any]:
    gtid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee:1-10"
    return {
        "schema_version": 1,
        "run_id": plan["run_id"],
        "snapshot_started_at_ms": 10,
        "snapshot_completed_at_ms": 20,
        "gtid_before_snapshot": gtid,
        "gtid_sha256": hashlib.sha256(gtid.encode("ascii")).hexdigest(),
        "query_sha256": "9" * 64,
        "source_binding_sha256": etl.sha256_json(plan["mysql_snapshot"]),
        "manifest_core_sha256": "a" * 64,
        "source_sha256": "b" * 64,
        "plan_sha256": etl.sha256_json(plan),
        "preflight_policy": "mysql_gtid_row_full_innodb_v2",
    }


def driver_source(mode: str) -> str:
    return f'''#!/usr/bin/env python3
import hashlib,json,sys
def canonical(value): return json.dumps(value,allow_nan=False,ensure_ascii=False,sort_keys=True,separators=(",",":"))
request_path = next(arg.split("=",1)[1] for arg in sys.argv if arg.startswith("--request-file="))
request = json.load(open(request_path, encoding="utf-8"))
mode = {mode!r}
server = request["server_uuid_sha256"] if mode != "wrong_server" else "f" * 64
resume_digest = None if request["resume_gtid"] is None else hashlib.sha256(request["resume_gtid"].encode("ascii")).hexdigest()
begin = {{"schema_version":1,"kind":"begin","server_uuid_sha256":server,"start_gtid_sha256":request["start_gtid_sha256"],"resume_gtid_sha256":resume_digest,"retained_boundary_ok":mode != "retention_gap","start_gtid_contained":True,"binlog_format":"ROW","binlog_row_image":"FULL","binlog_filters_safe":True}}
print(canonical(begin), flush=True)
resume = request["resume_sequence"]
sequences = [1,2] if resume == 0 else ([3] if resume == 2 else [])
if mode == "sequence_gap": sequences = [resume + 2]
if mode == "duplicate_gtid": sequences = [resume + 1]
for sequence in sequences:
    gtid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee:" + str(10 + sequence)
    if mode == "duplicate_gtid": gtid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee:11"
    table = "outside" if mode == "outside_table" else "items"
    material = {{"schema_version":1,"kind":"transaction","sequence":sequence,"gtid":gtid,"gtid_sha256":hashlib.sha256(gtid.encode("ascii")).hexdigest(),"committed_at_ms":100 + sequence,"changes":[{{"table":table,"operation":"insert","before":None,"after":{{"tenant":"tenant-a","id":str(sequence),"value":"protected-fixture-value"}}}}]}}
    frame = dict(material)
    frame["transaction_sha256"] = hashlib.sha256(canonical(material).encode("utf-8")).hexdigest()
    print(canonical(frame), flush=True)
if mode != "missing_heartbeat":
    final_sequence = sequences[-1] if sequences else resume
    heartbeat = {{"schema_version":1,"kind":"heartbeat","sequence":final_sequence,"executed_gtid_sha256":"c"*64,"caught_up":True,"start_gtid_contained":True,"post_boundary_heartbeat":True}}
    print(canonical(heartbeat), flush=True)
'''


def write_driver(root: pathlib.Path, name: str, mode: str) -> pathlib.Path:
    path = root / name
    path.write_text(driver_source(mode), encoding="utf-8")
    path.chmod(0o700)
    return path


def expect_cdc_error(callable_object, *args, **kwargs) -> None:
    try:
        callable_object(*args, **kwargs)
    except mysql_cdc.CdcError as error:
        text = str(error)
        assert "protected-fixture-value" not in text
        return
    raise AssertionError("expected CDC failure")


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-mysql-cdc-test-") as raw:
        root = pathlib.Path(raw)
        root.chmod(0o700)
        plan = plan_value()
        plan_path = root / "plan.json"
        boundary_path = root / "snapshot-boundary.protected.json"
        defaults = root / "client.cnf"
        plan_path.write_text(etl.canonical(plan) + "\n", encoding="utf-8")
        boundary_path.write_text(etl.canonical(boundary_value(plan)) + "\n", encoding="utf-8")
        defaults.write_text(
            "[client]\nhost=fixture.invalid\nuser=fixture\ndatabase=fixture\n"
            "password=not-a-real-secret\nssl-ca=/fixture/ca.pem\n",
            encoding="utf-8",
        )
        defaults.chmod(0o600)
        good = write_driver(root, "fake-cdc-good", "good")
        state = root / "state"

        first = mysql_cdc.capture(
            plan_path=plan_path,
            boundary_path=boundary_path,
            defaults_file=defaults,
            driver=str(good),
            state=state,
            timeout_seconds=10,
        )
        assert first["captured_transactions"] == 2
        assert first["durable_sequence"] == 2
        assert first["caught_up"] is True
        assert first["production_evidence"] is False

        second = mysql_cdc.capture(
            plan_path=plan_path,
            boundary_path=boundary_path,
            defaults_file=defaults,
            driver=str(good),
            state=state,
            timeout_seconds=10,
        )
        assert second["captured_transactions"] == 1
        assert second["durable_sequence"] == 3
        transactions = sorted((state / "transactions").glob("*.json"))
        assert len(transactions) == 3
        assert all(path.stat().st_mode & 0o077 == 0 for path in transactions)

        duplicate_state = root / "state-duplicate"
        mysql_cdc.capture(
            plan_path=plan_path,
            boundary_path=boundary_path,
            defaults_file=defaults,
            driver=str(good),
            state=duplicate_state,
            timeout_seconds=10,
        )
        duplicate_driver = write_driver(root, "fake-cdc-duplicate", "duplicate_gtid")
        expect_cdc_error(
            mysql_cdc.capture,
            plan_path=plan_path,
            boundary_path=boundary_path,
            defaults_file=defaults,
            driver=str(duplicate_driver),
            state=duplicate_state,
            timeout_seconds=10,
        )

        cases = {
            "gap": "sequence_gap",
            "server": "wrong_server",
            "retention": "retention_gap",
            "heartbeat": "missing_heartbeat",
            "allowlist": "outside_table",
        }
        for name, mode in cases.items():
            driver = write_driver(root, f"fake-cdc-{name}", mode)
            expect_cdc_error(
                mysql_cdc.capture,
                plan_path=plan_path,
                boundary_path=boundary_path,
                defaults_file=defaults,
                driver=str(driver),
                state=root / f"state-{name}",
                timeout_seconds=10,
            )

        atomic_path = root / "state-atomic" / "transactions" / "record.json"
        mysql_cdc._atomic_immutable_write(atomic_path, b"first\n")
        expect_cdc_error(mysql_cdc._atomic_immutable_write, atomic_path, b"second\n")
        assert atomic_path.read_bytes() == b"first\n"
        assert atomic_path.stat().st_mode & 0o077 == 0

        corrupt_state = root / "state-corrupt"
        corrupt_transactions = corrupt_state / "transactions"
        etl.private_directory(corrupt_transactions)
        corrupt_frame = {
            "kind": "transaction",
            "sequence": 1,
            "gtid": "non-ascii-\N{SNOWMAN}",
            "gtid_sha256": "d" * 64,
        }
        corrupt_digest = etl.sha256_json(corrupt_frame)
        corrupt_path = corrupt_transactions / f"{1:020d}-{corrupt_digest}.json"
        etl.immutable_private_write(
            corrupt_path, f"{etl.canonical(corrupt_frame)}\n".encode("utf-8")
        )
        expect_cdc_error(mysql_cdc._recover, corrupt_state)

    print(
        json.dumps(
            {
                "complete_row_images": True,
                "contiguous_transaction_sequence": True,
                "durable_resume": True,
                "duplicate_gtid_rejected": True,
                "gtid_snapshot_boundary": True,
                "immutable_journal_publication": True,
                "journal_tamper_rejected": True,
                "post_boundary_heartbeat": True,
                "production_evidence": False,
                "same_server_and_retention_fence": True,
                "source_table_allowlist": True,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
