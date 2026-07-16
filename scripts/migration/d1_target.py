#!/usr/bin/env python3
"""Fenced HTTP adapter for importing and reconciling Frame ETL bundles in D1.

The endpoint may be a local Worker served by ``wrangler dev`` or an approved
HTTPS operator API.  D1 is never treated as a local SQLite file: every chunk is
applied with an exclusive target generation and every reconciliation page is
read from one immutable snapshot id.  Responses are deliberately small and
strictly shaped so a stale Worker, partial page, or lost fence fails closed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import sqlite3
import stat
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Mapping, Sequence
from typing import Any, Protocol

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import etl


SCHEMA_VERSION = 1
MAX_REQUEST_BYTES = 64 * 1024 * 1024
MAX_RESPONSE_BYTES = 64 * 1024 * 1024
MAX_PAGE_ROWS = 10_000
MAX_AUTHORIZATION_BYTES = 16 * 1024


class D1TargetError(Exception):
    """A redacted, operator-safe target failure."""


class JsonTransport(Protocol):
    def request(self, method: str, path: str, body: Mapping[str, Any]) -> Mapping[str, Any]: ...


class _NoRedirectHandler(urllib.request.HTTPRedirectHandler):
    """Keep the operator credential on the reviewed origin."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):  # type: ignore[no-untyped-def]
        return None


def _exact(value: Any, fields: set[str], description: str) -> Mapping[str, Any]:
    if not isinstance(value, dict) or set(value) != fields:
        raise D1TargetError(f"D1 target returned an invalid {description}")
    return value


def _integer(value: Any, description: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= etl.MAX_WIRE_INTEGER:
        raise D1TargetError(f"D1 target returned an invalid {description}")
    return value


def _digest(value: Any, description: str) -> str:
    if not isinstance(value, str) or not etl.SHA256.fullmatch(value):
        raise D1TargetError(f"D1 target returned an invalid {description}")
    return value


def _token(value: Any, description: str) -> str:
    if not isinstance(value, str) or not etl.SAFE_LABEL.fullmatch(value):
        raise D1TargetError(f"D1 target returned an invalid {description}")
    return value


def _manifest_digest(bundle: pathlib.Path) -> str:
    return etl.sha256_bytes(
        etl.bounded_regular_bytes(bundle / "manifest.json", etl.MAX_MANIFEST_BYTES, "bundle manifest")
    )


def _private_authorization(path: pathlib.Path) -> str:
    descriptor: int | None = None
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_NOFOLLOW", 0))
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_mode & 0o077
            or not 1 <= metadata.st_size <= MAX_AUTHORIZATION_BYTES
        ):
            raise D1TargetError("D1 authorization file must be an owner-private regular file")
        with os.fdopen(descriptor, "rb", closefd=True) as handle:
            descriptor = None
            payload = handle.read(MAX_AUTHORIZATION_BYTES + 1)
    except OSError as error:
        raise D1TargetError("D1 authorization file is unavailable") from error
    finally:
        if descriptor is not None:
            os.close(descriptor)
    try:
        token = payload.decode("ascii").strip()
    except UnicodeDecodeError as error:
        raise D1TargetError("D1 authorization file is invalid") from error
    if (
        not 16 <= len(token) <= MAX_AUTHORIZATION_BYTES
        or any(not 0x21 <= ord(character) <= 0x7E for character in token)
    ):
        raise D1TargetError("D1 authorization file is invalid")
    return token


class HttpJsonTransport:
    """Bounded JSON transport for a Wrangler-local or approved remote Worker."""

    def __init__(self, endpoint: str, authorization_file: pathlib.Path, timeout_seconds: int) -> None:
        parsed = urllib.parse.urlsplit(endpoint)
        local = parsed.scheme == "http" and parsed.hostname in {"127.0.0.1", "localhost", "::1"}
        if parsed.scheme != "https" and not local:
            raise D1TargetError("D1 endpoint must use HTTPS or a loopback Wrangler address")
        if parsed.query or parsed.fragment or parsed.username or parsed.password:
            raise D1TargetError("D1 endpoint is invalid")
        if not 1 <= timeout_seconds <= 300:
            raise D1TargetError("D1 request timeout is outside its supported range")
        self.endpoint = endpoint.rstrip("/")
        self.authorization = _private_authorization(authorization_file)
        self.timeout_seconds = timeout_seconds
        self.opener = urllib.request.build_opener(_NoRedirectHandler())

    def request(self, method: str, path: str, body: Mapping[str, Any]) -> Mapping[str, Any]:
        if method not in {"POST"} or not path.startswith("/v1/migrations/etl/"):
            raise D1TargetError("D1 target request is outside the reviewed API surface")
        payload = etl.canonical(body).encode("utf-8")
        if len(payload) > MAX_REQUEST_BYTES:
            raise D1TargetError("D1 target request exceeds its supported bound")
        request = urllib.request.Request(
            f"{self.endpoint}{path}",
            data=payload,
            method=method,
            headers={
                "Authorization": f"Bearer {self.authorization}",
                "Content-Type": "application/json",
                "Accept": "application/json",
            },
        )
        try:
            with self.opener.open(request, timeout=self.timeout_seconds) as response:
                if response.status != 200:
                    raise D1TargetError("D1 target rejected the fenced request")
                encoded = response.read(MAX_RESPONSE_BYTES + 1)
        except (OSError, urllib.error.URLError, urllib.error.HTTPError) as error:
            raise D1TargetError("D1 target request failed safely") from error
        if len(encoded) > MAX_RESPONSE_BYTES:
            raise D1TargetError("D1 target response exceeds its supported bound")
        try:
            decoded = etl.strict_json(encoded.decode("utf-8"))
        except (UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
            raise D1TargetError("D1 target returned invalid JSON") from error
        if not isinstance(decoded, dict):
            raise D1TargetError("D1 target returned an invalid response")
        return decoded


class D1TargetAdapter:
    def __init__(self, transport: JsonTransport, bundle: pathlib.Path) -> None:
        self.transport = transport
        self.bundle = bundle
        self.manifest = etl.load_manifest(bundle)
        self.manifest_sha256 = _manifest_digest(bundle)
        self.fence_token: str | None = None
        self.snapshot_id: str | None = None
        self.generation: int | None = None
        self.phase = "new"

    def begin(self) -> None:
        if self.phase != "new":
            raise D1TargetError("D1 target fence has already been requested")
        response = _exact(
            self.transport.request(
                "POST",
                "/v1/migrations/etl/fences/begin",
                {
                    "schema_version": SCHEMA_VERSION,
                    "run_id": self.manifest["run_id"],
                    "manifest_sha256": self.manifest_sha256,
                    "target_migration": self.manifest["target_migration"],
                    "code_sha": self.manifest["code_sha"],
                    "window": self.manifest["window"],
                },
            ),
            {
                "schema_version",
                "fence_token",
                "snapshot_id",
                "target_generation",
                "target_migration",
                "run_id",
                "manifest_sha256",
                "expires_at_ms",
            },
            "fence response",
        )
        if (
            response["schema_version"] != SCHEMA_VERSION
            or response["target_migration"] != self.manifest["target_migration"]
            or response["run_id"] != self.manifest["run_id"]
            or _digest(response["manifest_sha256"], "manifest digest") != self.manifest_sha256
        ):
            raise D1TargetError("D1 target migration differs from the bundle")
        if _integer(response["expires_at_ms"], "fence expiry") <= int(time.time() * 1000):
            raise D1TargetError("D1 target fence is already expired")
        self.fence_token = _token(response["fence_token"], "fence token")
        self.snapshot_id = _token(response["snapshot_id"], "snapshot id")
        self.generation = _integer(response["target_generation"], "target generation")
        self.phase = "applying"

    def _fenced(self) -> tuple[str, str, int]:
        if self.fence_token is None or self.snapshot_id is None or self.generation is None:
            raise D1TargetError("D1 target fence has not been acquired")
        return self.fence_token, self.snapshot_id, self.generation

    def apply(self, *, max_rows_per_request: int = 1_000) -> Mapping[str, int]:
        if self.phase != "applying":
            raise D1TargetError("D1 target no longer accepts import pages")
        if not 1 <= max_rows_per_request <= MAX_PAGE_ROWS:
            raise D1TargetError("D1 target row batch is outside its supported range")
        fence, _snapshot, generation = self._fenced()
        applied = skipped = rows_applied = 0
        for table in self.manifest["tables"]:
            for tenant in table["tenants"]:
                previous_order: tuple[str | int, ...] | None = None
                for chunk in tenant["chunks"]:
                    rows = etl.read_chunk(self.bundle, chunk)
                    for row in rows:
                        current_order = etl._import_order_key(table, row)
                        try:
                            if previous_order is not None and current_order < previous_order:
                                raise D1TargetError("bundle rows violate their declared import order")
                        except TypeError as error:
                            raise D1TargetError("bundle import order has inconsistent scalar types") from error
                        previous_order = current_order
                    for offset in range(0, len(rows), max_rows_per_request):
                        page = rows[offset : offset + max_rows_per_request]
                        page_sha256 = etl.sha256_json(
                            {
                                "chunk_sha256": chunk["sha256"],
                                "offset": offset,
                                "rows": page,
                            }
                        )
                        response = _exact(
                            self.transport.request(
                                "POST",
                                "/v1/migrations/etl/chunks/apply",
                                {
                                    "schema_version": SCHEMA_VERSION,
                                    "fence_token": fence,
                                    "target_generation": generation,
                                    "run_id": self.manifest["run_id"],
                                    "manifest_sha256": self.manifest_sha256,
                                    "table": table["name"],
                                    "tenant_digest": tenant["tenant_digest"],
                                    "columns": table["columns"],
                                    "primary_key": table["primary_key"],
                                    "import_order": table.get("import_order", table["primary_key"]),
                                    "chunk_sha256": chunk["sha256"],
                                    "page_sha256": page_sha256,
                                    "offset": offset,
                                    "rows": page,
                                },
                            ),
                            {"schema_version", "status", "processed_rows", "page_sha256", "target_generation"},
                            "chunk response",
                        )
                        if (
                            response["schema_version"] != SCHEMA_VERSION
                            or response["status"] not in {"applied", "already_applied"}
                            or _integer(response["processed_rows"], "processed row count") != len(page)
                            or _digest(response["page_sha256"], "page digest") != page_sha256
                            or _integer(response["target_generation"], "target generation") != generation
                        ):
                            raise D1TargetError("D1 target chunk acknowledgement differs from the request")
                        if response["status"] == "applied":
                            applied += 1
                            rows_applied += len(page)
                        else:
                            skipped += 1
        return {"applied_pages": applied, "skipped_pages": skipped, "applied_rows": rows_applied}

    def reconcile(self, *, page_rows: int = 1_000) -> Mapping[str, Any]:
        if self.phase not in {"applying", "snapshot"}:
            raise D1TargetError("D1 target snapshot is unavailable in this phase")
        if not 1 <= page_rows <= MAX_PAGE_ROWS:
            raise D1TargetError("D1 snapshot page size is outside its supported range")
        fence, snapshot, generation = self._fenced()
        # The first snapshot request seals the reserved snapshot id.  Keep the
        # client in this phase even when reconciliation fails so an import can
        # never mutate the generation after pagination has begun.
        self.phase = "snapshot"
        sections: list[dict[str, Any]] = []
        unexplained = 0
        try:
            scratch_parent = self.bundle.parent.resolve(strict=True)
            metadata = scratch_parent.lstat()
        except OSError as error:
            raise D1TargetError("D1 reconciliation scratch parent is unavailable") from error
        if (
            not stat.S_ISDIR(metadata.st_mode)
            or stat.S_ISLNK(metadata.st_mode)
            or metadata.st_uid != os.getuid()
            or metadata.st_mode & 0o077
        ):
            raise D1TargetError("D1 reconciliation scratch parent must be owner private")
        with tempfile.TemporaryDirectory(prefix=".frame-d1-reconcile-", dir=scratch_parent) as raw:
            scratch = pathlib.Path(raw)
            scratch.chmod(0o700)
            connection = sqlite3.connect(scratch / "keys.sqlite")
            connection.execute("PRAGMA secure_delete=ON")
            connection.execute(
                "CREATE TABLE rows(side INTEGER NOT NULL, logical_key TEXT NOT NULL, row_hash TEXT NOT NULL, PRIMARY KEY(side,logical_key)) WITHOUT ROWID"
            )
            try:
                for table in self.manifest["tables"]:
                    connection.execute("DELETE FROM rows")
                    for tenant in table["tenants"]:
                        for chunk in tenant["chunks"]:
                            for row in etl.read_chunk(self.bundle, chunk):
                                key = etl.canonical([row[column] for column in table["primary_key"]])
                                try:
                                    connection.execute(
                                        "INSERT INTO rows VALUES (0,?,?)",
                                        (key, etl.sha256_json(row)),
                                    )
                                except sqlite3.IntegrityError as error:
                                    raise D1TargetError("bundle repeats a target reconciliation key") from error
                    cursor: str | None = None
                    seen_cursors: set[str] = set()
                    while True:
                        response = _exact(
                            self.transport.request(
                                "POST",
                                "/v1/migrations/etl/snapshots/page",
                                {
                                    "schema_version": SCHEMA_VERSION,
                                    "fence_token": fence,
                                    "snapshot_id": snapshot,
                                    "target_generation": generation,
                                    "manifest_sha256": self.manifest_sha256,
                                    "table": table["name"],
                                    "columns": table["columns"],
                                    "primary_key": table["primary_key"],
                                    "cursor": cursor,
                                    "page_rows": page_rows,
                                },
                            ),
                            {
                                "schema_version",
                                "snapshot_id",
                                "target_generation",
                                "rows",
                                "next_cursor",
                                "complete",
                            },
                            "snapshot page",
                        )
                        if (
                            response["schema_version"] != SCHEMA_VERSION
                            or response["snapshot_id"] != snapshot
                            or _integer(response["target_generation"], "snapshot generation") != generation
                            or not isinstance(response["rows"], list)
                            or len(response["rows"]) > page_rows
                            or not isinstance(response["complete"], bool)
                        ):
                            raise D1TargetError("D1 snapshot page crossed its fence")
                        for row in response["rows"]:
                            if not isinstance(row, dict) or set(row) != set(table["columns"]):
                                raise D1TargetError("D1 snapshot row differs from the manifest shape")
                            key = etl.canonical([row[column] for column in table["primary_key"]])
                            try:
                                connection.execute(
                                    "INSERT INTO rows VALUES (1,?,?)",
                                    (key, etl.sha256_json(row)),
                                )
                            except sqlite3.IntegrityError as error:
                                raise D1TargetError("D1 snapshot repeats a primary key") from error
                        next_cursor = response["next_cursor"]
                        if response["complete"]:
                            if next_cursor is not None:
                                raise D1TargetError("D1 complete snapshot page returned a cursor")
                            break
                        next_cursor = _token(next_cursor, "snapshot cursor")
                        if not response["rows"] or next_cursor in seen_cursors:
                            raise D1TargetError("D1 snapshot pagination did not advance")
                        seen_cursors.add(next_cursor)
                        cursor = next_cursor
                    expected_rows = connection.execute(
                        "SELECT COUNT(*) FROM rows WHERE side=0"
                    ).fetchone()[0]
                    actual_rows = connection.execute(
                        "SELECT COUNT(*) FROM rows WHERE side=1"
                    ).fetchone()[0]
                    missing = connection.execute(
                        "SELECT COUNT(*) FROM rows expected WHERE side=0 AND NOT EXISTS (SELECT 1 FROM rows actual WHERE actual.side=1 AND actual.logical_key=expected.logical_key)"
                    ).fetchone()[0]
                    extra = connection.execute(
                        "SELECT COUNT(*) FROM rows actual WHERE side=1 AND NOT EXISTS (SELECT 1 FROM rows expected WHERE expected.side=0 AND expected.logical_key=actual.logical_key)"
                    ).fetchone()[0]
                    changed = connection.execute(
                        "SELECT COUNT(*) FROM rows expected JOIN rows actual ON actual.side=1 AND actual.logical_key=expected.logical_key WHERE expected.side=0 AND expected.row_hash<>actual.row_hash"
                    ).fetchone()[0]
                    unexplained += missing + extra + changed
                    sections.append(
                        {
                            "table": table["name"],
                            "expected_rows": expected_rows,
                            "actual_rows": actual_rows,
                            "missing_primary_keys": missing,
                            "extra_primary_keys": extra,
                            "field_hash_mismatches": changed,
                        }
                    )
            finally:
                connection.close()
        verification = _exact(
            self.transport.request(
                "POST",
                "/v1/migrations/etl/snapshots/verify",
                {
                    "schema_version": SCHEMA_VERSION,
                    "fence_token": fence,
                    "snapshot_id": snapshot,
                    "target_generation": generation,
                    "manifest_sha256": self.manifest_sha256,
                },
            ),
            {"schema_version", "snapshot_id", "target_generation", "foreign_key_violations", "semantic_violations"},
            "snapshot verification",
        )
        if (
            verification["schema_version"] != SCHEMA_VERSION
            or verification["snapshot_id"] != snapshot
            or _integer(verification["target_generation"], "verification generation") != generation
        ):
            raise D1TargetError("D1 snapshot verification crossed its fence")
        relationships = _integer(verification["foreign_key_violations"], "foreign-key violation count")
        semantics = _integer(verification["semantic_violations"], "semantic violation count")
        unexplained += relationships + semantics
        report = {
            "schema_version": SCHEMA_VERSION,
            "run_id": self.manifest["run_id"],
            "manifest_sha256": self.manifest_sha256,
            "snapshot_id_sha256": hashlib.sha256(snapshot.encode("utf-8")).hexdigest(),
            "target_generation": generation,
            "tables": sections,
            "foreign_key_violations": relationships,
            "semantic_violations": semantics,
            "unexplained_mismatches": unexplained,
            "blocked_by_quarantine": self.manifest["reject_count"] > 0,
            "clean": unexplained == 0 and self.manifest["reject_count"] == 0,
            "production_evidence": False,
        }
        return report

    def finish(self, report: Mapping[str, Any]) -> None:
        if self.phase != "snapshot":
            raise D1TargetError("D1 target fence cannot finish before reconciliation")
        fence, snapshot, generation = self._fenced()
        exact_report = _exact(
            report,
            {
                "schema_version",
                "run_id",
                "manifest_sha256",
                "snapshot_id_sha256",
                "target_generation",
                "tables",
                "foreign_key_violations",
                "semantic_violations",
                "unexplained_mismatches",
                "blocked_by_quarantine",
                "clean",
                "production_evidence",
            },
            "reconciliation report",
        )
        if (
            exact_report["schema_version"] != SCHEMA_VERSION
            or exact_report["run_id"] != self.manifest["run_id"]
            or exact_report["manifest_sha256"] != self.manifest_sha256
            or exact_report["snapshot_id_sha256"]
            != hashlib.sha256(snapshot.encode("utf-8")).hexdigest()
            or _integer(exact_report["target_generation"], "report generation") != generation
            or _integer(exact_report["foreign_key_violations"], "foreign-key violation count") != 0
            or _integer(exact_report["semantic_violations"], "semantic violation count") != 0
            or _integer(exact_report["unexplained_mismatches"], "unexplained mismatch count") != 0
            or exact_report["blocked_by_quarantine"] is not False
            or exact_report["clean"] is not True
            or exact_report["production_evidence"] is not False
            or not isinstance(exact_report["tables"], list)
            or [section.get("table") for section in exact_report["tables"] if isinstance(section, dict)]
            != [table["name"] for table in self.manifest["tables"]]
        ):
            raise D1TargetError("D1 target fence cannot finish with mismatches")
        for section in exact_report["tables"]:
            section = _exact(
                section,
                {
                    "table",
                    "expected_rows",
                    "actual_rows",
                    "missing_primary_keys",
                    "extra_primary_keys",
                    "field_hash_mismatches",
                },
                "reconciliation table section",
            )
            expected = _integer(section["expected_rows"], "expected row count")
            actual = _integer(section["actual_rows"], "actual row count")
            if (
                expected != actual
                or _integer(section["missing_primary_keys"], "missing primary-key count") != 0
                or _integer(section["extra_primary_keys"], "extra primary-key count") != 0
                or _integer(section["field_hash_mismatches"], "field mismatch count") != 0
            ):
                raise D1TargetError("D1 target fence cannot finish with mismatches")
        report_sha256 = etl.sha256_json(exact_report)
        response = _exact(
            self.transport.request(
                "POST",
                "/v1/migrations/etl/fences/finish",
                {
                    "schema_version": SCHEMA_VERSION,
                    "fence_token": fence,
                    "snapshot_id": snapshot,
                    "target_generation": generation,
                    "manifest_sha256": self.manifest_sha256,
                    "report_sha256": report_sha256,
                },
            ),
            {"schema_version", "status", "target_generation", "report_sha256"},
            "fence completion",
        )
        if (
            response["schema_version"] != SCHEMA_VERSION
            or response["status"] != "matched"
            or _integer(response["target_generation"], "completion generation") != generation
            or _digest(response["report_sha256"], "completion report digest") != report_sha256
        ):
            raise D1TargetError("D1 target did not durably acknowledge reconciliation")
        self.phase = "finished"


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    root.add_argument("--bundle", type=pathlib.Path, required=True)
    root.add_argument("--endpoint", required=True)
    root.add_argument("--authorization-file", type=pathlib.Path, required=True)
    root.add_argument("--report", type=pathlib.Path, required=True)
    root.add_argument("--timeout-seconds", type=int, default=60)
    root.add_argument("--page-rows", type=int, default=1_000)
    root.add_argument("--max-rows-per-request", type=int, default=1_000)
    return root


def main(arguments: Sequence[str] | None = None) -> int:
    args = parser().parse_args(arguments)
    try:
        transport = HttpJsonTransport(args.endpoint, args.authorization_file, args.timeout_seconds)
        adapter = D1TargetAdapter(transport, args.bundle)
        adapter.begin()
        applied = adapter.apply(max_rows_per_request=args.max_rows_per_request)
        report = adapter.reconcile(page_rows=args.page_rows)
        if report["clean"]:
            adapter.finish(report)
        etl.atomic_private_write(args.report, f"{etl.canonical(report)}\n".encode("utf-8"))
        print(
            f"D1 target applied {applied['applied_pages']} pages and reconciled "
            f"{len(report['tables'])} tables; unexplained mismatches: "
            f"{report['unexplained_mismatches']}"
        )
        return 0 if report["clean"] else 4
    except (D1TargetError, etl.EtlError, OSError, sqlite3.Error) as error:
        print(f"D1 target failed safely: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
