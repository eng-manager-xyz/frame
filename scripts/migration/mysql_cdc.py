#!/usr/bin/env python3
"""Resumable GTID-bound MySQL row-change capture for Frame metadata ETL.

This adapter invokes a reviewed CDC driver through a strict canonical-NDJSON
protocol.  The driver owns MySQL's replication protocol and row decoding; this
module owns snapshot/server binding, complete transaction framing, durable
resume, table allowlisting, contiguous delivery, and a terminal caught-up
heartbeat.  Row values stay in owner-private journal files and never enter the
shareable report.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import selectors
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from collections.abc import Iterator, Mapping, Sequence
from typing import Any

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import etl
import mysql_snapshot


SCHEMA_VERSION = 1
MAX_FRAME_BYTES = 16 * 1024 * 1024
MAX_STDERR_BYTES = 1024 * 1024
MAX_CAPTURE_BYTES = 2 * 1024 * 1024 * 1024
MAX_CHANGES_PER_TRANSACTION = 100_000
DEFAULT_TIMEOUT_SECONDS = 14_400
BEGIN_FIELDS = {
    "schema_version",
    "kind",
    "server_uuid_sha256",
    "start_gtid_sha256",
    "resume_gtid_sha256",
    "retained_boundary_ok",
    "start_gtid_contained",
    "binlog_format",
    "binlog_row_image",
    "binlog_filters_safe",
}


class CdcError(Exception):
    """A stable CDC failure which contains neither credentials nor row values."""


def _driver_executable(value: str) -> pathlib.Path:
    resolved = shutil.which(value)
    if resolved is None:
        raise CdcError("approved CDC driver executable was not found")
    path = pathlib.Path(resolved).resolve(strict=True)
    metadata = path.stat()
    if (
        not stat.S_ISREG(metadata.st_mode)
        or metadata.st_mode & 0o111 == 0
        or metadata.st_uid not in (0, os.getuid())
        or metadata.st_mode & 0o022
    ):
        raise CdcError("approved CDC driver executable is invalid")
    return path


def _load_boundary(path: pathlib.Path) -> Mapping[str, Any]:
    try:
        payload = etl.bounded_regular_bytes(
            path, etl.MAX_SNAPSHOT_ATTESTATION_BYTES, "protected snapshot boundary"
        )
        decoded = etl.strict_json(payload.decode("utf-8"))
    except (UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
        raise CdcError("protected snapshot boundary is invalid") from error
    required = {
        "schema_version",
        "run_id",
        "snapshot_started_at_ms",
        "snapshot_completed_at_ms",
        "gtid_before_snapshot",
        "gtid_sha256",
        "query_sha256",
        "source_binding_sha256",
        "manifest_core_sha256",
        "source_sha256",
        "plan_sha256",
        "preflight_policy",
    }
    if (
        not isinstance(decoded, dict)
        or set(decoded) != required
        or payload != f"{etl.canonical(decoded)}\n".encode("utf-8")
        or decoded["schema_version"] != SCHEMA_VERSION
        or decoded["preflight_policy"] != "mysql_gtid_row_full_innodb_v2"
    ):
        raise CdcError("protected snapshot boundary is invalid")
    gtid = decoded["gtid_before_snapshot"]
    try:
        encoded_gtid = gtid.encode("ascii") if isinstance(gtid, str) else b""
    except UnicodeEncodeError as error:
        raise CdcError("protected snapshot GTID boundary is invalid") from error
    if not encoded_gtid or len(encoded_gtid) > mysql_snapshot.MAX_GTID_BYTES:
        raise CdcError("protected snapshot GTID boundary is invalid")
    if hashlib.sha256(encoded_gtid).hexdigest() != decoded["gtid_sha256"]:
        raise CdcError("protected snapshot GTID boundary digest differs")
    return decoded


def _source_tables(plan: etl.Plan) -> list[str]:
    raw_tables = plan.raw.get("tables")
    if not isinstance(raw_tables, list):
        raise CdcError("CDC plan tables are invalid")
    result: list[str] = []
    for raw in raw_tables:
        if not isinstance(raw, dict):
            raise CdcError("CDC plan table is invalid")
        table = raw.get("source_table", raw.get("name"))
        if not isinstance(table, str) or not etl.IDENTIFIER.fullmatch(table):
            raise CdcError("CDC plan source table is invalid")
        if table not in result:
            result.append(table)
    return result


def _transaction_path(state: pathlib.Path, sequence: int, digest: str) -> pathlib.Path:
    return state / "transactions" / f"{sequence:020d}-{digest}.json"


def _atomic_immutable_write(path: pathlib.Path, payload: bytes) -> None:
    """Publish one journal record atomically without replacing an existing file."""

    etl.private_directory(path.parent)
    descriptor, temporary = tempfile.mkstemp(prefix=".cdc-transaction-", dir=path.parent)
    temporary_path = pathlib.Path(temporary)
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(payload)
            handle.flush()
            os.fsync(handle.fileno())
        try:
            mysql_snapshot._rename_noreplace(temporary_path, path)
        except mysql_snapshot.SnapshotError as error:
            raise CdcError("CDC journal publication conflicted safely") from error
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except BaseException:
        temporary_path.unlink(missing_ok=True)
        raise


def _recover(state: pathlib.Path) -> tuple[int, str | None, int, set[str]]:
    transactions = state / "transactions"
    etl.private_directory(transactions)
    sequence = 0
    last_gtid: str | None = None
    total_bytes = 0
    seen_gtids: set[str] = set()
    for path in sorted(transactions.iterdir()):
        if not path.is_file() or path.is_symlink():
            raise CdcError("CDC journal contains an unsafe entry")
        try:
            prefix, digest_with_suffix = path.name.split("-", maxsplit=1)
            if not digest_with_suffix.endswith(".json"):
                raise ValueError
            digest = digest_with_suffix.removesuffix(".json")
            ordinal = int(prefix)
        except ValueError as error:
            raise CdcError("CDC journal entry name is invalid") from error
        if (
            path.name != f"{ordinal:020d}-{digest}.json"
            or ordinal != sequence + 1
            or not etl.SHA256.fullmatch(digest)
        ):
            raise CdcError("CDC journal sequence is not contiguous")
        payload = etl.bounded_regular_bytes(path, MAX_FRAME_BYTES, "CDC journal transaction")
        total_bytes += len(payload)
        if total_bytes > MAX_CAPTURE_BYTES:
            raise CdcError("CDC journal exceeds its supported size")
        try:
            frame = etl.strict_json(payload.decode("utf-8"))
        except (UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
            raise CdcError("CDC journal transaction is invalid") from error
        if (
            not isinstance(frame, dict)
            or payload != f"{etl.canonical(frame)}\n".encode("utf-8")
            or frame.get("kind") != "transaction"
            or frame.get("sequence") != ordinal
            or etl.sha256_json(frame) != digest
        ):
            raise CdcError("CDC journal transaction digest differs")
        last_gtid = frame.get("gtid")
        if not isinstance(last_gtid, str):
            raise CdcError("CDC journal GTID is invalid")
        try:
            encoded_gtid = last_gtid.encode("ascii")
        except UnicodeEncodeError as error:
            raise CdcError("CDC journal GTID is invalid") from error
        if not encoded_gtid or len(encoded_gtid) > mysql_snapshot.MAX_GTID_BYTES:
            raise CdcError("CDC journal GTID is invalid")
        gtid_sha256 = hashlib.sha256(encoded_gtid).hexdigest()
        if frame.get("gtid_sha256") != gtid_sha256 or gtid_sha256 in seen_gtids:
            raise CdcError("CDC journal repeats or corrupts a GTID")
        seen_gtids.add(gtid_sha256)
        sequence = ordinal
    return sequence, last_gtid, total_bytes, seen_gtids


def _frames(
    executable: pathlib.Path,
    defaults_file: pathlib.Path,
    request_file: pathlib.Path,
    timeout_seconds: int,
) -> Iterator[Mapping[str, Any]]:
    if not 1 <= timeout_seconds <= DEFAULT_TIMEOUT_SECONDS:
        raise CdcError("CDC timeout is outside its supported range")
    process: subprocess.Popen[bytes] | None = None
    pending = bytearray()
    stderr_bytes = 0
    output_bytes = 0
    started = time.monotonic()
    try:
        process = subprocess.Popen(
            [
                str(executable),
                f"--defaults-file={defaults_file}",
                f"--request-file={request_file}",
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            close_fds=True,
            env={"TZ": "UTC", "LANG": os.environ.get("LANG", "C")},
        )
        assert process.stdout is not None and process.stderr is not None
        selector = selectors.DefaultSelector()
        selector.register(process.stdout, selectors.EVENT_READ, "stdout")
        selector.register(process.stderr, selectors.EVENT_READ, "stderr")
        while selector.get_map():
            if time.monotonic() - started > timeout_seconds:
                raise CdcError("CDC driver exceeded its bounded execution time")
            events = selector.select(timeout=1)
            if not events and process.poll() is not None:
                events = [(item, selectors.EVENT_READ) for item in selector.get_map().values()]
            for key, _mask in events:
                chunk = os.read(key.fd, 64 * 1024)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                if key.data == "stderr":
                    stderr_bytes += len(chunk)
                    if stderr_bytes > MAX_STDERR_BYTES:
                        raise CdcError("CDC driver diagnostics exceed their supported bound")
                    continue
                output_bytes += len(chunk)
                if output_bytes > MAX_CAPTURE_BYTES:
                    raise CdcError("CDC capture exceeds its supported size")
                pending.extend(chunk)
                if len(pending) > MAX_FRAME_BYTES and b"\n" not in pending:
                    raise CdcError("CDC frame exceeds its supported size")
                while b"\n" in pending:
                    raw, _, remainder = pending.partition(b"\n")
                    pending = bytearray(remainder)
                    if raw.endswith(b"\r") or not raw or len(raw) > MAX_FRAME_BYTES:
                        raise CdcError("CDC driver returned invalid framing")
                    try:
                        decoded = etl.strict_json(raw.decode("utf-8"))
                    except (UnicodeDecodeError, ValueError, json.JSONDecodeError) as error:
                        raise CdcError("CDC driver returned invalid JSON") from error
                    if not isinstance(decoded, dict) or raw != etl.canonical(decoded).encode("utf-8"):
                        raise CdcError("CDC driver frame is not canonical")
                    yield decoded
        if pending:
            raise CdcError("CDC driver returned a truncated frame")
        if process.wait(timeout=5) != 0:
            raise CdcError("CDC driver failed safely")
    except BaseException:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        raise


def _validate_change(change: Any, allowed_tables: set[str]) -> None:
    if not isinstance(change, dict) or set(change) != {"table", "operation", "before", "after"}:
        raise CdcError("CDC transaction contains an invalid change")
    if change["table"] not in allowed_tables or change["operation"] not in {"insert", "update", "delete"}:
        raise CdcError("CDC transaction is outside the plan allowlist")
    before = change["before"]
    after = change["after"]
    if (
        (before is not None and not isinstance(before, dict))
        or (after is not None and not isinstance(after, dict))
        or (change["operation"] == "insert" and (before is not None or after is None))
        or (change["operation"] == "delete" and (before is None or after is not None))
        or (change["operation"] == "update" and (before is None or after is None))
    ):
        raise CdcError("CDC transaction row image is incomplete")


def capture(
    *,
    plan_path: pathlib.Path,
    boundary_path: pathlib.Path,
    defaults_file: pathlib.Path,
    driver: str,
    state: pathlib.Path,
    timeout_seconds: int,
) -> Mapping[str, Any]:
    plan = etl.load_plan(plan_path)
    boundary = _load_boundary(boundary_path)
    try:
        source_binding = mysql_snapshot._snapshot_expectations(plan)
    except mysql_snapshot.SnapshotError as error:
        raise CdcError("CDC plan lacks an exact MySQL source binding") from error
    if boundary["source_binding_sha256"] != etl.sha256_json(source_binding):
        raise CdcError("CDC plan source binding differs from the snapshot")
    if (
        boundary["run_id"] != plan.run_id
        or boundary["plan_sha256"] != etl.sha256_json(plan.raw)
    ):
        raise CdcError("CDC plan identity differs from the snapshot")
    server_uuid_sha256 = source_binding.get("server_uuid_sha256")
    if not isinstance(server_uuid_sha256, str) or not etl.SHA256.fullmatch(server_uuid_sha256):
        raise CdcError("CDC plan server identity is invalid")
    defaults_payload = mysql_snapshot._private_defaults_file(defaults_file)
    executable = _driver_executable(driver)
    etl.private_directory(state)
    sequence, resume_gtid, journal_bytes, seen_gtids = _recover(state)
    allowed_tables = _source_tables(plan)
    request = {
        "schema_version": SCHEMA_VERSION,
        "run_id": plan.run_id,
        "server_uuid_sha256": server_uuid_sha256,
        "start_gtid": boundary["gtid_before_snapshot"],
        "start_gtid_sha256": boundary["gtid_sha256"],
        "resume_sequence": sequence,
        "resume_gtid": resume_gtid,
        "allowed_tables": allowed_tables,
    }
    begin_seen = False
    heartbeat: Mapping[str, Any] | None = None
    captured = 0
    with tempfile.TemporaryDirectory(prefix=".frame-mysql-cdc-", dir=state) as raw:
        scratch = pathlib.Path(raw)
        scratch.chmod(0o700)
        staged_defaults = scratch / "client.cnf"
        request_file = scratch / "request.json"
        etl.immutable_private_write(staged_defaults, defaults_payload)
        etl.immutable_private_write(request_file, f"{etl.canonical(request)}\n".encode("utf-8"))
        for frame in _frames(executable, staged_defaults, request_file, timeout_seconds):
            kind = frame.get("kind")
            if not begin_seen:
                if set(frame) != BEGIN_FIELDS or kind != "begin" or frame.get("schema_version") != SCHEMA_VERSION:
                    raise CdcError("CDC driver did not begin with a source fence")
                expected_resume = (
                    hashlib.sha256(resume_gtid.encode("ascii")).hexdigest()
                    if resume_gtid is not None
                    else None
                )
                if (
                    frame.get("server_uuid_sha256") != server_uuid_sha256
                    or frame.get("start_gtid_sha256") != boundary["gtid_sha256"]
                    or frame.get("resume_gtid_sha256") != expected_resume
                    or frame.get("retained_boundary_ok") is not True
                    or frame.get("start_gtid_contained") is not True
                    or frame.get("binlog_format") != "ROW"
                    or frame.get("binlog_row_image") != "FULL"
                    or frame.get("binlog_filters_safe") is not True
                ):
                    raise CdcError("CDC source cannot prove a no-gap snapshot boundary")
                begin_seen = True
                continue
            if kind == "heartbeat":
                if heartbeat is not None or set(frame) != {
                    "schema_version", "kind", "sequence", "executed_gtid_sha256",
                    "caught_up", "start_gtid_contained", "post_boundary_heartbeat"
                }:
                    raise CdcError("CDC heartbeat is invalid")
                heartbeat = frame
                continue
            if heartbeat is not None:
                raise CdcError("CDC driver returned transactions after catch-up")
            if kind != "transaction" or set(frame) != {
                "schema_version", "kind", "sequence", "gtid", "gtid_sha256",
                "committed_at_ms", "changes", "transaction_sha256"
            }:
                raise CdcError("CDC transaction frame is invalid")
            next_sequence = sequence + 1
            if frame.get("schema_version") != SCHEMA_VERSION or frame.get("sequence") != next_sequence:
                raise CdcError("CDC transaction sequence contains a gap")
            gtid = frame.get("gtid")
            try:
                encoded_gtid = gtid.encode("ascii") if isinstance(gtid, str) else b""
            except UnicodeEncodeError as error:
                raise CdcError("CDC transaction GTID is invalid") from error
            if not encoded_gtid or len(encoded_gtid) > mysql_snapshot.MAX_GTID_BYTES:
                raise CdcError("CDC transaction GTID is invalid")
            gtid_sha256 = hashlib.sha256(encoded_gtid).hexdigest()
            if frame.get("gtid_sha256") != gtid_sha256 or gtid_sha256 in seen_gtids:
                raise CdcError("CDC transaction GTID is repeated or corrupt")
            changes = frame.get("changes")
            if not isinstance(changes, list) or not 1 <= len(changes) <= MAX_CHANGES_PER_TRANSACTION:
                raise CdcError("CDC transaction change count is invalid")
            for change in changes:
                _validate_change(change, set(allowed_tables))
            material = dict(frame)
            claimed = material.pop("transaction_sha256")
            if claimed != etl.sha256_json(material):
                raise CdcError("CDC transaction digest differs")
            committed_at_ms = frame.get("committed_at_ms")
            if (
                isinstance(committed_at_ms, bool)
                or not isinstance(committed_at_ms, int)
                or not 0 <= committed_at_ms <= etl.MAX_WIRE_INTEGER
            ):
                raise CdcError("CDC transaction commit time is invalid")
            payload = f"{etl.canonical(frame)}\n".encode("utf-8")
            journal_digest = etl.sha256_json(frame)
            destination = _transaction_path(state, next_sequence, journal_digest)
            if journal_bytes + len(payload) > MAX_CAPTURE_BYTES:
                raise CdcError("CDC journal exceeds its supported size")
            _atomic_immutable_write(destination, payload)
            journal_bytes += len(payload)
            checkpoint = {
                "schema_version": SCHEMA_VERSION,
                "run_id": plan.run_id,
                "server_uuid_sha256": server_uuid_sha256,
                "start_gtid_sha256": boundary["gtid_sha256"],
                "sequence": next_sequence,
                "last_gtid": gtid,
                "last_gtid_sha256": gtid_sha256,
                "journal_sha256": journal_digest,
            }
            etl.atomic_private_write(
                state / "checkpoint.protected.json",
                f"{etl.canonical(checkpoint)}\n".encode("utf-8"),
            )
            sequence = next_sequence
            resume_gtid = gtid
            seen_gtids.add(gtid_sha256)
            captured += 1
    if not begin_seen or heartbeat is None:
        raise CdcError("CDC driver did not return a terminal heartbeat")
    if (
        heartbeat.get("schema_version") != SCHEMA_VERSION
        or heartbeat.get("sequence") != sequence
        or heartbeat.get("caught_up") is not True
        or heartbeat.get("start_gtid_contained") is not True
        or heartbeat.get("post_boundary_heartbeat") is not True
        or not isinstance(heartbeat.get("executed_gtid_sha256"), str)
        or not etl.SHA256.fullmatch(heartbeat["executed_gtid_sha256"])
    ):
        raise CdcError("CDC terminal heartbeat cannot prove catch-up")
    return {
        "schema_version": SCHEMA_VERSION,
        "run_id": plan.run_id,
        "server_uuid_sha256": server_uuid_sha256,
        "start_gtid_sha256": boundary["gtid_sha256"],
        "executed_gtid_sha256": heartbeat["executed_gtid_sha256"],
        "captured_transactions": captured,
        "durable_sequence": sequence,
        "journal_bytes": journal_bytes,
        "caught_up": True,
        "post_boundary_heartbeat": True,
        "production_evidence": False,
    }


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    root.add_argument("--plan", type=pathlib.Path, required=True)
    root.add_argument("--snapshot-boundary", type=pathlib.Path, required=True)
    root.add_argument("--defaults-file", type=pathlib.Path, required=True)
    root.add_argument("--driver", required=True)
    root.add_argument("--state", type=pathlib.Path, required=True)
    root.add_argument("--report", type=pathlib.Path, required=True)
    root.add_argument("--timeout-seconds", type=int, default=DEFAULT_TIMEOUT_SECONDS)
    return root


def main(arguments: Sequence[str] | None = None) -> int:
    args = parser().parse_args(arguments)
    try:
        report = capture(
            plan_path=args.plan,
            boundary_path=args.snapshot_boundary,
            defaults_file=args.defaults_file,
            driver=args.driver,
            state=args.state,
            timeout_seconds=args.timeout_seconds,
        )
        etl.atomic_private_write(args.report, f"{etl.canonical(report)}\n".encode("utf-8"))
        print(
            f"captured {report['captured_transactions']} transactions; "
            f"durable sequence {report['durable_sequence']}; caught up"
        )
        return 0
    except (CdcError, etl.EtlError, OSError) as error:
        print(f"MySQL CDC failed safely: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
