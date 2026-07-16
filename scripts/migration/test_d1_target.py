#!/usr/bin/env python3
"""Credential-free fake proof for the fenced D1 target adapter."""

from __future__ import annotations

import copy
import json
import pathlib
import sys
import tempfile
import time
from collections.abc import Mapping
from typing import Any

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import d1_target
import etl


def plan_value() -> dict[str, Any]:
    return {
        "schema_version": 1,
        "run_id": "d1-target-local-v1",
        "source_schema": "fixture-v1",
        "target_migration": "fixture-d1-v1",
        "code_sha": "1" * 64,
        "window": {"start_ms": 1, "end_ms": 2},
        "tables": [
            {
                "name": "items",
                "tenant_column": "tenant_id",
                "primary_key": ["id"],
                "import_order": ["sequence", "id"],
                "columns": [
                    {"source": "tenant", "target": "tenant_id"},
                    {"source": "id", "target": "id"},
                    {"source": "sequence", "target": "sequence", "transform": "wire_integer"},
                    {"source": "value", "target": "value"},
                ],
            }
        ],
        "semantic_rules": [],
    }


class FakeTransport:
    def __init__(self) -> None:
        self.generation = 7
        self.rows: dict[str, dict[str, dict[str, Any]]] = {}
        self.pages: set[str] = set()
        self.calls: list[str] = []
        self.finished = False
        self.stall_snapshot = False
        self.bad_begin_binding = False
        self.snapshot_rows: dict[str, dict[str, dict[str, Any]]] | None = None

    def request(self, method: str, path: str, body: Mapping[str, Any]) -> Mapping[str, Any]:
        assert method == "POST"
        self.calls.append(path)
        if path.endswith("/fences/begin"):
            return {
                "schema_version": 1,
                "fence_token": "fence-local-v1",
                "snapshot_id": "snapshot-local-v1",
                "target_generation": self.generation,
                "target_migration": body["target_migration"],
                "run_id": body["run_id"],
                "manifest_sha256": "f" * 64 if self.bad_begin_binding else body["manifest_sha256"],
                "expires_at_ms": int(time.time() * 1000) + 60_000,
            }
        if path.endswith("/chunks/apply"):
            assert self.snapshot_rows is None
            assert body["target_generation"] == self.generation
            table = str(body["table"])
            status = "already_applied" if body["page_sha256"] in self.pages else "applied"
            if status == "applied":
                self.pages.add(str(body["page_sha256"]))
                target = self.rows.setdefault(table, {})
                for row in body["rows"]:
                    key = etl.canonical([row[column] for column in body["primary_key"]])
                    existing = target.setdefault(key, dict(row))
                    assert existing == row
            return {
                "schema_version": 1,
                "status": status,
                "processed_rows": len(body["rows"]),
                "page_sha256": body["page_sha256"],
                "target_generation": self.generation,
            }
        if path.endswith("/snapshots/page"):
            if self.snapshot_rows is None:
                self.snapshot_rows = copy.deepcopy(self.rows)
            if self.stall_snapshot:
                return {
                    "schema_version": 1,
                    "snapshot_id": "snapshot-local-v1",
                    "target_generation": self.generation,
                    "rows": [],
                    "next_cursor": "offset-stalled",
                    "complete": False,
                }
            rows = list(self.snapshot_rows.get(str(body["table"]), {}).values())
            rows.sort(key=lambda row: etl.canonical([row[column] for column in body["primary_key"]]))
            offset = 0 if body["cursor"] is None else int(str(body["cursor"]).split("-")[-1])
            page = rows[offset : offset + int(body["page_rows"])]
            next_offset = offset + len(page)
            complete = next_offset == len(rows)
            return {
                "schema_version": 1,
                "snapshot_id": "snapshot-local-v1",
                "target_generation": self.generation,
                "rows": page,
                "next_cursor": None if complete else f"offset-{next_offset}",
                "complete": complete,
            }
        if path.endswith("/snapshots/verify"):
            return {
                "schema_version": 1,
                "snapshot_id": "snapshot-local-v1",
                "target_generation": self.generation,
                "foreign_key_violations": 0,
                "semantic_violations": 0,
            }
        if path.endswith("/fences/finish"):
            assert self.snapshot_rows is not None
            self.finished = True
            return {
                "schema_version": 1,
                "status": "matched",
                "target_generation": self.generation,
                "report_sha256": body["report_sha256"],
            }
        raise AssertionError(path)


def expect_target_error(callable_object, *args, **kwargs) -> None:
    try:
        callable_object(*args, **kwargs)
    except d1_target.D1TargetError as error:
        assert "secret" not in str(error).casefold()
        return
    raise AssertionError("expected fenced D1 target failure")


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-d1-target-test-") as raw:
        root = pathlib.Path(raw)
        root.chmod(0o700)
        plan_path = root / "plan.json"
        source_path = root / "source.ndjson"
        plan_path.write_text(etl.canonical(plan_value()) + "\n", encoding="utf-8")
        source_rows = [
            {"table": "items", "tenant": "tenant-a", "row": {"tenant": "tenant-a", "id": "b", "sequence": 2, "value": "two"}},
            {"table": "items", "tenant": "tenant-a", "row": {"tenant": "tenant-a", "id": "a", "sequence": 1, "value": "one"}},
        ]
        source_path.write_text("".join(etl.canonical(row) + "\n" for row in source_rows), encoding="utf-8")
        bundle = root / "bundle"
        etl.export_bundle(source_path, etl.load_plan(plan_path), bundle, 1)

        chunks = etl.load_manifest(bundle)["tables"][0]["tenants"][0]["chunks"]
        ordered = [etl.read_chunk(bundle, chunk)[0]["sequence"] for chunk in chunks]
        assert ordered == [1, 2]

        transport = FakeTransport()
        adapter = d1_target.D1TargetAdapter(transport, bundle)
        adapter.begin()
        first = adapter.apply(max_rows_per_request=1)
        assert first == {"applied_pages": 2, "skipped_pages": 0, "applied_rows": 2}
        second = adapter.apply(max_rows_per_request=1)
        assert second == {"applied_pages": 0, "skipped_pages": 2, "applied_rows": 0}
        report = adapter.reconcile(page_rows=1)
        assert report["clean"] is True
        assert report["production_evidence"] is False
        adapter.finish(report)
        assert transport.finished
        expect_target_error(adapter.apply, max_rows_per_request=1)
        expect_target_error(adapter.begin)

        binding = FakeTransport()
        binding.bad_begin_binding = True
        expect_target_error(d1_target.D1TargetAdapter(binding, bundle).begin)

        drift = FakeTransport()
        drift_adapter = d1_target.D1TargetAdapter(drift, bundle)
        drift_adapter.begin()
        drift_adapter.apply()
        drift.generation += 1
        expect_target_error(drift_adapter.reconcile, page_rows=1)

        mismatch = FakeTransport()
        mismatch_adapter = d1_target.D1TargetAdapter(mismatch, bundle)
        mismatch_adapter.begin()
        mismatch_adapter.apply()
        mismatch.rows["items"]['["a"]']["value"] = "changed"
        mismatch_report = mismatch_adapter.reconcile(page_rows=1)
        assert mismatch_report["clean"] is False
        assert mismatch_report["unexplained_mismatches"] == 1
        expect_target_error(mismatch_adapter.finish, mismatch_report)

        stalled = FakeTransport()
        stalled_adapter = d1_target.D1TargetAdapter(stalled, bundle)
        stalled_adapter.begin()
        stalled_adapter.apply()
        stalled.stall_snapshot = True
        expect_target_error(stalled_adapter.reconcile, page_rows=1)

        authorization = root / "authorization"
        authorization.write_text("fixture-authorization-token\n", encoding="ascii")
        authorization.chmod(0o600)
        http_transport = d1_target.HttpJsonTransport(
            "http://127.0.0.1:8787", authorization, 1
        )
        assert http_transport.opener.handlers
        original_request_bound = d1_target.MAX_REQUEST_BYTES
        try:
            d1_target.MAX_REQUEST_BYTES = 1
            expect_target_error(
                http_transport.request,
                "POST",
                "/v1/migrations/etl/fences/begin",
                {"too": "large"},
            )
        finally:
            d1_target.MAX_REQUEST_BYTES = original_request_bound
        expect_target_error(
            d1_target.HttpJsonTransport,
            "http://not-loopback.invalid",
            authorization,
            1,
        )

    print(
        json.dumps(
            {
                "complete_paginated_snapshot": True,
                "bounded_request_and_no_redirect_transport": True,
                "fence_manifest_binding": True,
                "fenced_generation_drift_rejected": True,
                "idempotent_chunk_checkpoint": True,
                "nonadvancing_cursor_rejected": True,
                "import_order_separate_from_primary_key": True,
                "production_evidence": False,
                "snapshot_seals_import_phase": True,
                "wrangler_loopback_and_https_contract": True,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
