#!/usr/bin/env python3
"""Validate the credential-free pinned-Cap business ETL plan contract."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import pathlib
import sqlite3
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "fixtures/etl/v1/cap-business-plan-contract.json"
EXECUTABLE_PLAN = ROOT / "fixtures/etl/v1/cap-business-plan.json"
SOURCE_INVENTORY = ROOT / "fixtures/parity/v1/business-cap-schema-v1.json"
PARITY = ROOT / "fixtures/parity/v1/business-cap-schema-v1.json"
MIGRATIONS = ROOT / "apps/control-plane/migrations"
CAP_COMMIT = "6ba69561ac86b8efdb17616d6727f9638015546b"
CAP_SCHEMA_SHA256 = "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9"
CAP_COLUMN_CONTRACT_SHA256 = (
    "1eaab20cf10260f727aaef539d51bdd933157814f7cbd327dddb40bee6ae4fce"
)
EXPECTED_IDENTIFIERS = {
    "videos": {"id", "ownerId", "orgId", "bucket", "storageIntegrationId", "folderId"},
    "video_edits": {"videoId"},
    "shared_videos": {"id", "videoId", "folderId", "organizationId", "sharedByUserId"},
    "comments": {"id", "authorId", "videoId", "parentCommentId"},
    "notifications": {"id", "orgId", "recipientId"},
    "messenger_conversations": {"id", "userId", "takeoverByUserId"},
    "messenger_messages": {"id", "conversationId", "userId"},
    "messenger_support_emails": {"id", "conversationId", "userId"},
    "s3_buckets": {"id", "ownerId", "organizationId"},
    "storage_integrations": {"id", "ownerId", "organizationId"},
    "storage_objects": {"id", "integrationId", "ownerId", "videoId"},
    "video_uploads": {"videoId"},
    "imported_videos": {"id", "orgId"},
    "developer_apps": {"id", "ownerId"},
    "developer_app_domains": {"id", "appId"},
    "developer_api_keys": {"id", "appId"},
    "developer_videos": {"id", "appId"},
    "developer_credit_accounts": {"id", "appId", "ownerId"},
    "developer_credit_transactions": {"id", "accountId"},
    "developer_daily_storage_snapshots": {"id", "appId"},
}


def load_mapper():
    path = ROOT / "scripts/migration/cap_id_map.py"
    specification = importlib.util.spec_from_file_location("frame_cap_plan_mapper", path)
    if specification is None or specification.loader is None:
        raise RuntimeError("could not load Cap ID mapper")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


def migrate() -> sqlite3.Connection:
    database = sqlite3.connect(":memory:")
    database.execute("PRAGMA foreign_keys=ON")
    for path in sorted(MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql")):
        database.executescript(path.read_text(encoding="utf-8"))
    return database


def main() -> int:
    contract = json.loads(CONTRACT.read_text(encoding="utf-8"))
    source_inventory = json.loads(SOURCE_INVENTORY.read_text(encoding="utf-8"))
    parity = json.loads(PARITY.read_text(encoding="utf-8"))
    assert contract["contract_version"] == 1
    assert contract["execution_mode"] == "credential_free_template"
    assert CAP_COMMIT in contract["source_reference"]
    assert contract["transform"] == "cap_nanoid_uuid_v1"
    assert contract["external_id_dependencies"] == [
        "users.id",
        "organizations.id",
        "folders.id",
    ]
    assert source_inventory["schema_version"] == 1
    assert source_inventory["corpus_version"] == "parity-v1"
    assert source_inventory["source_reference"] == contract["source_reference"]
    source_tables = {
        item["source_table"]: set(item["source_columns"])
        for item in source_inventory["source_tables"]
    }
    assert set(source_tables) == set(EXPECTED_IDENTIFIERS)
    for table_name, identifiers in EXPECTED_IDENTIFIERS.items():
        assert identifiers.issubset(source_tables[table_name])

    tables = {item["source_table"]: item for item in contract["tables"]}
    assert set(tables) == set(EXPECTED_IDENTIFIERS)
    assert len(tables) == 20
    assert contract["derived_aggregates"] == [
        {"aggregate": "usage_ledger", "provenance": "frame_derived", "source_table": None}
    ]

    assert parity["schema_version"] == 1
    assert parity["corpus_version"] == "parity-v1"
    assert parity["source_reference"] == (
        f"CapSoftware/Cap@{CAP_COMMIT}:packages/database/schema.ts"
    )
    assert parity["source_file_sha256"] == CAP_SCHEMA_SHA256
    assert parity["source_column_contract_sha256"] == CAP_COLUMN_CONTRACT_SHA256
    parity_tables = {item["source_table"]: item for item in parity["source_tables"]}
    assert set(parity_tables) == set(EXPECTED_IDENTIFIERS)
    assert len(parity_tables) == 20
    column_contract = {
        name: parity_tables[name]["source_columns"] for name in sorted(parity_tables)
    }
    column_contract_digest = hashlib.sha256(
        json.dumps(
            column_contract, sort_keys=True, separators=(",", ":")
        ).encode("utf-8")
    ).hexdigest()
    assert column_contract_digest == CAP_COLUMN_CONTRACT_SHA256
    drift_count = 0
    for table_name, expected_identifiers in EXPECTED_IDENTIFIERS.items():
        parity_table = parity_tables[table_name]
        source_columns = parity_table["source_columns"]
        assert source_columns
        assert len(source_columns) == len(set(source_columns))
        assert expected_identifiers <= set(source_columns)
        assert parity_table["target_contract"]
        intentional_drifts = parity_table["intentional_drifts"]
        assert intentional_drifts
        for drift in intentional_drifts:
            drift_count += 1
            assert set(drift) == {"kind", "source", "target", "rationale"}
            assert all(isinstance(value, str) and value for value in drift.values())
    assert drift_count == 39
    assert parity["derived_aggregates"] == [
        {
            "aggregate": "usage_ledger",
            "provenance": "frame_derived",
            "rationale": "auditable usage facts absent from pinned Cap schema",
        }
    ]

    database = migrate()
    mapped = {
        row[0]
        for row in database.execute("SELECT source_table FROM business_source_table_map_v1")
    }
    assert mapped == set(EXPECTED_IDENTIFIERS)
    assert database.execute(
        "SELECT aggregate,provenance FROM business_derived_aggregate_map_v1"
    ).fetchall() == [("usage_ledger", "frame_derived")]

    known_sources = {
        f"{table}.{column}"
        for table, columns in EXPECTED_IDENTIFIERS.items()
        for column in columns
    }
    external = set(contract["external_id_dependencies"])
    transformed = 0
    for table_name, expected_columns in EXPECTED_IDENTIFIERS.items():
        table = tables[table_name]
        identifiers = {item["source"]: item for item in table["identifiers"]}
        assert set(identifiers) == expected_columns
        for column, identifier in identifiers.items():
            assert column in parity_tables[table_name]["source_columns"]
            transformed += 1
            assert identifier["transform"] == "cap_nanoid_uuid_v1"
            assert "options" not in identifier
            targets = identifier["target"]
            assert targets or identifier.get("target_disposition")
            for target in targets:
                target_table, target_column = target.split(".", 1)
                columns = {
                    row[1]
                    for row in database.execute(
                        f'PRAGMA table_info("{target_table}")'
                    ).fetchall()
                }
                assert target_column in columns, f"unknown target {target}"
            reference = identifier.get("references")
            if reference is not None:
                assert reference in known_sources or reference in external
                referenced_table, referenced_column = reference.split(".", 1)
                if referenced_table in tables:
                    assert (
                        tables[referenced_table]["identifiers"][
                            next(
                                index
                                for index, item in enumerate(
                                    tables[referenced_table]["identifiers"]
                                )
                                if item["source"] == referenced_column
                            )
                        ]["transform"]
                        == "cap_nanoid_uuid_v1"
                    )

    mapper = load_mapper()
    source = "0123456789abcde"
    expected = "2a6a8a87-d5ca-8c83-8666-2e92c2a69404"
    for table in contract["tables"]:
        for identifier in table["identifiers"]:
            assert mapper.map_cap_nanoid(source) == expected

    sys.path.insert(0, str(ROOT / "scripts/migration"))
    import etl
    import mysql_snapshot

    executable = etl.load_plan(EXECUTABLE_PLAN)
    executable_sources = [
        raw.get("source_table", raw["name"])
        for raw in executable.raw["tables"]
    ]
    assert len(executable.tables) == 21
    assert set(executable_sources) == set(EXPECTED_IDENTIFIERS)
    assert len(executable_sources) > len(set(executable_sources))
    assert executable_sources.count("notifications") == 2
    assert executable.raw["mapping_contract_sha256"] == hashlib.sha256(
        CONTRACT.read_bytes()
    ).hexdigest()
    assert executable.raw["scope_contract"] == {
        "table": "frame_business_tenant_scope_v1",
        "primary_key": "sourceId",
        "tenant_column": "organizationId",
        "requirement": "one approved organization for every owner-scoped source root",
    }
    target_tables = set()
    for table in executable.tables:
        target_tables.add(table.target_name)
        actual_columns = {
            row[1]
            for row in database.execute(
                f'PRAGMA table_info("{table.target_name}")'
            ).fetchall()
        }
        assert {column.target for column in table.columns} <= actual_columns
    assert "outbox_events" in target_tables
    assert "business_messenger_legacy_quarantine_v1" in target_tables
    sql = mysql_snapshot.snapshot_sql(executable)
    assert "ROW_NUMBER() OVER (PARTITION BY" in sql
    assert "`balanceAfterMicroCredits`" in sql
    assert "`frame_business_tenant_scope_v1`" in sql
    edit = next(table for table in executable.tables if table.name == "video_edits")
    edit_id_targets = [
        column.target
        for column in edit.columns
        if column.input_sources() == ("video_id",)
    ]
    assert edit_id_targets[:2] == ["id", "video_id"]
    credit = next(
        table for table in executable.tables
        if table.name == "developer_credit_transactions"
    )
    assert credit.import_order == ("account_id", "ledger_sequence", "id")
    storage = next(table for table in executable.tables if table.name == "s3_buckets")
    capabilities = next(
        column for column in storage.columns if column.target == "capabilities_json"
    )
    capabilities_checksum = next(
        column for column in storage.columns if column.target == "capabilities_checksum"
    )
    capability_values = ("us-east-1", "https://storage.invalid", "fixture")
    canonical_capabilities = etl.transform_value(capabilities, capability_values)
    assert etl.transform_value(capabilities_checksum, capability_values) == hashlib.sha256(
        canonical_capabilities.encode("utf-8")
    ).hexdigest()
    share = next(table for table in executable.tables if table.name == "shared_videos")
    sharing_mode = next(
        column for column in share.columns if column.target == "sharing_mode"
    )
    assert etl.transform_value(sharing_mode, None) == "organization"
    assert etl.transform_value(sharing_mode, "folder") == "space"
    notifications_targets = {
        table.target_name
        for table, raw in zip(executable.tables, executable.raw["tables"], strict=True)
        if raw.get("source_table", raw["name"]) == "notifications"
    }
    assert notifications_targets == {"notifications", "outbox_events"}

    print(
        json.dumps(
            {
                "derived_aggregates": 1,
                "documented_intentional_drifts": drift_count,
                "source_tables": len(tables),
                "executable_plan_streams": len(executable.tables),
                "transformed_pk_fk_identifiers": transformed,
                "status": "ok",
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
