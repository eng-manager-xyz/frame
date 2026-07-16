#!/usr/bin/env python3
"""Exercise AggregateRepository through the compiled local Worker.

Wrangler CLI is used only to create the isolated database, load controlled
fixtures, inspect final invariants, and retain D1 query plans. Repository
behavior is invoked over HTTP through the Rust/Wasm Worker surface.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import http.client
import json
import os
import pathlib
import re
import secrets
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
from collections.abc import Iterable, Sequence
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
QUERIES = ROOT / "apps" / "control-plane" / "queries"
CONFIG = ROOT / "apps" / "control-plane" / "wrangler.local.toml"
REPOSITORY_SOURCE = ROOT / "apps" / "control-plane" / "src" / "repository.rs"
CONFORMANCE_SOURCE = (
    ROOT / "apps" / "control-plane" / "src" / "repository_conformance.rs"
)
ROUTING_SOURCE = ROOT / "apps" / "control-plane" / "src" / "routing.rs"
LIB_SOURCE = ROOT / "apps" / "control-plane" / "src" / "lib.rs"
DATABASE = "frame-local"
WRANGLER_VERSION = "4.111.0"
MAX_PARAMETERS = 100
MAX_PAGE_SIZE = 100
ANSI = re.compile(r"\x1b\[[0-9;]*m")
PLACEHOLDER = re.compile(r"\?([1-9][0-9]*)")

USER_A = "018f47a6-7b1c-7f55-8f39-8f8a86900101"
ORG_A = "018f47a6-7b1c-7f55-8f39-8f8a86900102"
VIDEO_A = "018f47a6-7b1c-7f55-8f39-8f8a86900104"
UPLOAD_A = "018f47a6-7b1c-7f55-8f39-8f8a86900111"
JOB_A = "018f47a6-7b1c-7f55-8f39-8f8a86900112"
USER_B = "018f47a6-7b1c-7f55-8f39-8f8a86900201"
ORG_B = "018f47a6-7b1c-7f55-8f39-8f8a86900202"
VIDEO_B = "018f47a6-7b1c-7f55-8f39-8f8a86900204"
UPLOAD_B = "018f47a6-7b1c-7f55-8f39-8f8a86900211"
JOB_B = "018f47a6-7b1c-7f55-8f39-8f8a86900212"
CONTENTION_VIDEO = "018f47a6-7b1c-7f55-8f39-8f8a86900304"
CONFORMANCE_PATH = "/__frame/local/repository-conformance"
TOKEN_HEADER = "x-frame-repository-conformance-token"
INITIAL_EVENT_FINGERPRINT = (
    "daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9"
)


class ConformanceFailure(RuntimeError):
    """A stable assertion that never includes SQL, bindings, or provider text."""


def sql_literal(value: object) -> str:
    if value is None:
        return "NULL"
    if isinstance(value, bool):
        return "1" if value else "0"
    if isinstance(value, int):
        return str(value)
    if not isinstance(value, str) or "\x00" in value:
        raise ValueError("unsupported controlled fixture value")
    return "'" + value.replace("'", "''") + "'"


def render_bound_sql(sql: str, bindings: Sequence[object]) -> str:
    """Render controlled bindings for CLI-only parity and plan inspection."""
    seen: set[int] = set()

    def replace(match: re.Match[str]) -> str:
        index = int(match.group(1))
        if index > len(bindings):
            raise ConformanceFailure("query references an unavailable binding")
        seen.add(index)
        return sql_literal(bindings[index - 1])

    rendered = PLACEHOLDER.sub(replace, sql)
    if seen != set(range(1, len(bindings) + 1)):
        raise ConformanceFailure("query does not consume every declared binding")
    return rendered.strip()


def canonical_rows(rows: Iterable[sqlite3.Row | dict[str, Any]]) -> list[dict[str, Any]]:
    normalized = [dict(row) for row in rows]
    return json.loads(json.dumps(normalized, sort_keys=True, separators=(",", ":")))


def query_text(name: str) -> str:
    sql = (QUERIES / name).read_text(encoding="utf-8")
    if ";" in sql or not PLACEHOLDER.search(sql):
        raise ConformanceFailure("checked-in query contract is invalid")
    return sql


def migration_files() -> list[pathlib.Path]:
    files = sorted(MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql"))
    if [int(path.name[:4]) for path in files] != list(range(1, len(files) + 1)):
        raise ConformanceFailure("migration sequence is not contiguous")
    return files


def fixture_video_id(index: int) -> str:
    return f"018f47a6-7b1c-7f55-8f39-{0x100000 + index:012x}"


def fixture_statements() -> list[str]:
    statements = [
        "PRAGMA foreign_keys = ON",
        f"INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES ({sql_literal(USER_A)},'a@frame.invalid','A',1700000000000,1700000000000)",
        f"INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES ({sql_literal(USER_B)},'b@frame.invalid','B',1700000000000,1700000000000)",
        f"INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,revision) VALUES ({sql_literal(ORG_A)},{sql_literal(USER_A)},'Tenant A','active','{{}}',1700000000000,1700000000000,2)",
        f"INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,revision) VALUES ({sql_literal(ORG_B)},{sql_literal(USER_B)},'Tenant B','active','{{}}',1700000000000,1700000000000,3)",
        f"INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms,revision) VALUES ({sql_literal(ORG_A)},{sql_literal(USER_A)},'owner','active',1,1700000000000,1700000000000,0)",
        f"INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms,revision) VALUES ({sql_literal(ORG_B)},{sql_literal(USER_B)},'owner','active',1,1700000000000,1700000000000,0)",
        f"INSERT INTO videos(id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id,privacy,metadata_json,revision) VALUES ({sql_literal(VIDEO_A)},{sql_literal(USER_A)},'Found A','ready',1700000000100,1700000000100,{sql_literal(ORG_A)},'private',NULL,4)",
        f"INSERT INTO videos(id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id,privacy,metadata_json,revision) VALUES ({sql_literal(VIDEO_B)},{sql_literal(USER_B)},'Found B','ready',1700000000100,1700000000100,{sql_literal(ORG_B)},'private',NULL,8)",
        f"INSERT INTO videos(id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id,privacy,metadata_json,revision) VALUES ({sql_literal(CONTENTION_VIDEO)},{sql_literal(USER_A)},'Contention','ready',1700000000200,1700000000200,{sql_literal(ORG_A)},'private',NULL,10)",
        f"INSERT INTO video_uploads(id,organization_id,video_id,state,expected_bytes,received_bytes,idempotency_key,source_object_key,source_version,content_type,checksum_sha256,created_at_ms,updated_at_ms,revision,event_sequence,event_fingerprint) VALUES ({sql_literal(UPLOAD_A)},{sql_literal(ORG_A)},{sql_literal(VIDEO_A)},'initiated',42,0,'upload-a-key',{sql_literal(f'tenants/{ORG_A}/videos/{VIDEO_A}/source/v1/source.webm')},1,'video/webm',NULL,1700000000000,1700000000000,0,0,{sql_literal(INITIAL_EVENT_FINGERPRINT)})",
        f"UPDATE video_uploads SET state='uploading',received_bytes=21,updated_at_ms=1700000000001,revision=1,event_sequence=1,event_fingerprint={'e' * 64!r} WHERE id={sql_literal(UPLOAD_A)}",
        f"UPDATE video_uploads SET state='finalizing',received_bytes=42,updated_at_ms=1700000000002,revision=2,event_sequence=2,event_fingerprint={'f' * 64!r} WHERE id={sql_literal(UPLOAD_A)}",
        f"UPDATE video_uploads SET state='complete',checksum_sha256={'a' * 64!r},updated_at_ms=1700000000003,revision=3,event_sequence=3,event_fingerprint={'b' * 64!r} WHERE id={sql_literal(UPLOAD_A)}",
        f"INSERT INTO video_uploads(id,organization_id,video_id,state,expected_bytes,received_bytes,idempotency_key,source_object_key,source_version,content_type,checksum_sha256,created_at_ms,updated_at_ms,revision,event_sequence,event_fingerprint) VALUES ({sql_literal(UPLOAD_B)},{sql_literal(ORG_B)},{sql_literal(VIDEO_B)},'initiated',42,0,'upload-b-key',{sql_literal(f'tenants/{ORG_B}/videos/{VIDEO_B}/source/v1/source.webm')},1,'video/webm',NULL,1700000000000,1700000000000,0,0,{sql_literal(INITIAL_EVENT_FINGERPRINT)})",
        f"INSERT INTO media_jobs(id,video_id,kind,state,idempotency_key,attempt,payload_json,created_at_ms,updated_at_ms,organization_id,selected_executor,source_version,profile_version,output_object_key,progress_basis_points,revision) VALUES ({sql_literal(JOB_A)},{sql_literal(VIDEO_A)},'frame','succeeded','job-a-key',1,'{{\"profile\":\"thumbnail_v1\"}}',1700000000000,1700000000001,{sql_literal(ORG_A)},'native_gstreamer',1,1,{sql_literal(f'tenants/{ORG_A}/videos/{VIDEO_A}/derivatives/thumbnail_v1/{"a" * 64}')},10000,2)",
        f"INSERT INTO media_jobs(id,video_id,kind,state,idempotency_key,attempt,payload_json,created_at_ms,updated_at_ms,organization_id,selected_executor,source_version,profile_version,output_object_key,revision) VALUES ({sql_literal(JOB_B)},{sql_literal(VIDEO_B)},'frame','queued','job-b-key',0,'{{\"profile\":\"thumbnail_v1\"}}',1700000000000,1700000000000,{sql_literal(ORG_B)},'native_gstreamer',1,1,{sql_literal(f'tenants/{ORG_B}/videos/{VIDEO_B}/derivatives/thumbnail_v1/{"b" * 64}')},0)",
    ]
    for index in range(205):
        video_id = fixture_video_id(index)
        created_at = 1_700_001_000_000 + index // 3
        statements.append(
            "INSERT INTO videos(id,owner_id,title,state,created_at_ms,updated_at_ms,"
            "organization_id,privacy,metadata_json,revision) VALUES "
            f"({sql_literal(video_id)},{sql_literal(USER_A)},'Page {index:03d}','ready',"
            f"{created_at},{created_at},{sql_literal(ORG_A)},'private',NULL,0)"
        )
    rejected_key = f"repository-video-title:{ORG_A}:repository-constraint-0001"
    statements.append(
        "CREATE TRIGGER conformance_reject_title_outbox BEFORE INSERT ON outbox_events "
        f"WHEN NEW.deduplication_key={sql_literal(rejected_key)} "
        "BEGIN SELECT RAISE(ABORT,'conformance outbox constraint'); END"
    )
    return statements


def apply_reference() -> sqlite3.Connection:
    database = sqlite3.connect(":memory:")
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    for path in migration_files():
        database.executescript(path.read_text(encoding="utf-8"))
    for statement in fixture_statements():
        database.execute(statement)
    database.commit()
    return database


def verify_rust_contract() -> None:
    source = REPOSITORY_SOURCE.read_text(encoding="utf-8")
    for name, expected in {
        "MAX_PAGE_SIZE": MAX_PAGE_SIZE,
        "MAX_D1_BOUND_PARAMETERS": MAX_PARAMETERS,
        "MAX_BULK_IDENTIFIERS": 1_000,
    }.items():
        match = re.search(rf"pub const {name}: usize = ([0-9_]+);", source)
        if match is None or int(match.group(1).replace("_", "")) != expected:
            raise ConformanceFailure("Rust/Python repository limit drifted")
    included = set(re.findall(r'include_str!\("\.\./queries/([^\"]+)"\)', source))
    expected_queries = {
        "video_for_mutation.sql",
        "upload_by_id.sql",
        "media_job_by_id.sql",
        "native_worker_job_by_id.sql",
        "organization_snapshot.sql",
        "video_page.sql",
        "video_page_after.sql",
        "video_title_command.sql",
        "video_title_apply.sql",
    }
    if included != expected_queries:
        raise ConformanceFailure("runtime and conformance query inventories diverged")
    surface = "\n".join(
        path.read_text(encoding="utf-8")
        for path in (CONFORMANCE_SOURCE, ROUTING_SOURCE, LIB_SOURCE)
    )
    for marker in (
        CONFORMANCE_PATH,
        "FRAME_REPOSITORY_CONFORMANCE_TOKEN",
        "update_video_title",
        "with_query_timeout_ms(database, 0)",
        "config.production() || !valid_repository_conformance_target",
    ):
        if marker not in surface:
            raise ConformanceFailure("compiled Worker conformance surface drifted")


class WranglerD1:
    def __init__(self, command: list[str], state: pathlib.Path) -> None:
        self.command = command
        self.state = state
        self.environment = os.environ.copy()
        self.environment.update(
            {
                "CI": "true",
                "NO_COLOR": "1",
                "WRANGLER_LOG_PATH": str(state / "wrangler-cli.log"),
            }
        )

    def run(
        self,
        arguments: Sequence[str],
        *,
        timeout: float = 60,
        check: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        process = subprocess.run(
            [*self.command, *arguments],
            cwd=ROOT,
            env=self.environment,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
        if check and process.returncode != 0:
            if arguments[:2] == ["d1", "migrations"]:
                phase = "migration_apply"
            elif "--file" in arguments:
                phase = "fixture_load"
            elif "--command" in arguments:
                phase = "bounded_query"
            else:
                phase = "unknown"
            raise ConformanceFailure(f"local Wrangler D1 command failed ({phase})")
        return process

    def d1_base(self) -> list[str]:
        return [
            "d1",
            "execute",
            DATABASE,
            "--local",
            "--persist-to",
            str(self.state),
            "--config",
            str(CONFIG),
            "--json",
        ]

    def migrate(self) -> None:
        self.run(
            [
                "d1",
                "migrations",
                "apply",
                DATABASE,
                "--local",
                "--persist-to",
                str(self.state),
                "--config",
                str(CONFIG),
            ],
            timeout=90,
        )

    def execute_file(self, path: pathlib.Path) -> None:
        process = self.run([*self.d1_base()[:-1], "--file", str(path), "--json"])
        payload = parse_wrangler_json(process.stdout)
        if not isinstance(payload, list) or not all(
            isinstance(item, dict) and item.get("success") is True for item in payload
        ):
            raise ConformanceFailure("local D1 fixture load failed")

    def execute(self, sql: str) -> list[dict[str, Any]]:
        process = self.run([*self.d1_base(), "--command", sql])
        payload = parse_wrangler_json(process.stdout)
        if not isinstance(payload, list) or not all(
            isinstance(item, dict) and item.get("success") is True for item in payload
        ):
            raise ConformanceFailure("local D1 returned an unsuccessful result")
        return payload

    def query(self, sql: str) -> list[dict[str, Any]]:
        payload = self.execute(sql)
        if len(payload) != 1 or not isinstance(payload[0].get("results"), list):
            raise ConformanceFailure("local D1 query result shape changed")
        return canonical_rows(payload[0]["results"])


def parse_wrangler_json(output: str) -> Any:
    clean = ANSI.sub("", output).strip()
    try:
        return json.loads(clean)
    except json.JSONDecodeError as error:
        raise ConformanceFailure("Wrangler did not return valid JSON") from error


def detect_wrangler(explicit: str | None) -> list[str]:
    command = (
        (["node", explicit] if explicit and explicit.endswith(".js") else [explicit])
        if explicit
        else ["npx", "--yes", f"wrangler@{WRANGLER_VERSION}"]
    )
    environment = os.environ.copy()
    environment.update(
        {"NO_COLOR": "1", "WRANGLER_LOG_PATH": "/tmp/frame-wrangler-version.log"}
    )
    version = subprocess.run(
        [*command, "--version"],
        cwd=ROOT,
        env=environment,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    if version.returncode != 0 or ANSI.sub("", version.stdout).strip() != WRANGLER_VERSION:
        raise ConformanceFailure(f"Wrangler {WRANGLER_VERSION} is required")
    return command


def refuse_external_authority() -> None:
    forbidden = [
        name
        for name in ("CLOUDFLARE_API_TOKEN", "CLOUDFLARE_ACCOUNT_ID", "DATABASE_URL")
        if os.environ.get(name)
    ]
    if os.environ.get("FRAME_DEPLOYMENT") == "production":
        forbidden.append("FRAME_DEPLOYMENT")
    if forbidden:
        raise ConformanceFailure("local conformance refused external authority variables")


def write_fixture(path: pathlib.Path) -> None:
    path.write_text(";\n".join(fixture_statements()) + ";\n", encoding="utf-8")


def assert_reference_query_parity(
    reference: sqlite3.Connection, d1: WranglerD1
) -> None:
    cases = [
        ("video_for_mutation.sql", [VIDEO_A, ORG_A, USER_A]),
        ("video_for_mutation.sql", [VIDEO_B, ORG_A, USER_A]),
        ("upload_by_id.sql", [UPLOAD_A, ORG_A]),
        ("upload_by_id.sql", [UPLOAD_B, ORG_A]),
        ("media_job_by_id.sql", [JOB_A, ORG_A]),
        ("native_worker_job_by_id.sql", [JOB_A, ORG_A]),
        ("organization_snapshot.sql", [ORG_A]),
    ]
    for name, bindings in cases:
        sql = query_text(name)
        expected = canonical_rows(reference.execute(sql, bindings).fetchall())
        actual = d1.query(render_bound_sql(sql, bindings))
        if actual != expected:
            raise ConformanceFailure("reference and local D1 read semantics diverged")
    expected_ids = [
        row["id"]
        for row in reference.execute(
            "SELECT id FROM videos WHERE organization_id=?1 AND deleted_at_ms IS NULL "
            "ORDER BY created_at_ms DESC,id DESC",
            (ORG_A,),
        ).fetchall()
    ]
    observed: list[str] = []
    cursor: tuple[int, str] | None = None
    while True:
        if cursor is None:
            sql = query_text("video_page.sql")
            bindings: list[object] = [ORG_A, 101]
        else:
            sql = query_text("video_page_after.sql")
            bindings = [ORG_A, cursor[0], cursor[1], 101]
        rows = d1.query(render_bound_sql(sql, bindings))
        page = rows[:100]
        observed.extend(str(row["id"]) for row in page)
        if len(rows) <= 100:
            break
        cursor = (int(page[-1]["created_at_ms"]), str(page[-1]["id"]))
    if observed != expected_ids or len(observed) != len(set(observed)):
        raise ConformanceFailure("reference/D1 keyset traversal diverged")


def reference_write_operation() -> tuple[str, Sequence[object]]:
    key = "repository-success-0001"
    title = "Repository \"Applied\" – O'Brien"
    digest_payload = json.dumps(
        {
            "tenant_id": ORG_A,
            "video_id": VIDEO_A,
            "actor_id": USER_A,
            "expected_revision": 4,
            "title": title,
        },
        ensure_ascii=False,
        separators=(",", ":"),
    )
    digest = hashlib.sha256(
        b"repository_video_title_v1\0" + digest_payload.encode()
    ).hexdigest()
    response = json.dumps(
        {
            "schema_version": 1,
            "video_id": VIDEO_A,
            "title": title,
            "revision": 5,
        },
        ensure_ascii=False,
        separators=(",", ":"),
    )
    payload_checksum = hashlib.sha256(response.encode("utf-8")).hexdigest()
    reservation = "018f47a6-7b1c-7f55-8f39-8f8a86904101"
    outbox = "018f47a6-7b1c-7f55-8f39-8f8a86904102"
    operation = "018f47a6-7b1c-7f55-8f39-8f8a86904103"
    dedup = f"repository-video-title:{ORG_A}:{key}"
    return (
        query_text("video_title_apply.sql"),
        [
            operation,
            ORG_A,
            VIDEO_A,
            USER_A,
            key,
            digest,
            reservation,
            outbox,
            dedup,
            4,
            title,
            response,
            response,
            1_700_100_000_000,
            1_700_186_400_000,
            payload_checksum,
            INITIAL_EVENT_FINGERPRINT,
        ],
    )


def assert_reference_write_operation(reference: sqlite3.Connection) -> None:
    """Prove the ignored envelope retains trigger effects, then roll back."""
    sql, bindings = reference_write_operation()
    reference.execute("BEGIN IMMEDIATE")
    try:
        reference.execute(sql, bindings)
        row = reference.execute(
            "SELECT v.title,v.revision,c.response_status,"
            "(SELECT COUNT(*) FROM outbox_events e WHERE e.aggregate_id=v.id) AS outbox_count,"
            "(SELECT COUNT(*) FROM repository_video_title_operations) AS operation_count "
            "FROM videos v JOIN command_idempotency c ON c.organization_id=v.organization_id "
            "WHERE v.id=?1 AND c.idempotency_key='repository-success-0001'",
            (VIDEO_A,),
        ).fetchone()
        if row is None or dict(row) != {
            "title": "Repository \"Applied\" – O'Brien",
            "revision": 5,
            "response_status": 200,
            "outbox_count": 1,
            "operation_count": 0,
        }:
            raise ConformanceFailure("reference trigger-guarded operation invariant changed")
    except sqlite3.Error as error:
        raise ConformanceFailure("reference rejected checked-in write operation") from error
    finally:
        reference.rollback()


def query_plans(d1: WranglerD1) -> dict[str, list[str]]:
    initial = render_bound_sql(query_text("video_page.sql"), [ORG_A, 26])
    after = render_bound_sql(
        query_text("video_page_after.sql"),
        [ORG_A, 1_700_001_000_040, fixture_video_id(120), 26],
    )
    cases = {
        "video_page": initial,
        "video_page_after": after,
        "upload_by_id": render_bound_sql(
            query_text("upload_by_id.sql"), [UPLOAD_A, ORG_A]
        ),
        "organization_snapshot": render_bound_sql(
            query_text("organization_snapshot.sql"), [ORG_A]
        ),
    }
    plans: dict[str, list[str]] = {}
    for name, sql in cases.items():
        details = [
            str(row.get("detail", ""))
            for row in d1.query("EXPLAIN QUERY PLAN " + sql)
        ]
        if not details or any("SCAN videos" in detail for detail in details):
            raise ConformanceFailure("repository plan regressed to a video-table scan")
        plans[name] = details
    if not any("videos_org_active_page_idx" in detail for detail in plans["video_page"]):
        raise ConformanceFailure("initial page did not use its named index")
    if not any(
        "videos_org_active_page_idx" in detail
        and re.search(r"\(created_at_ms\s*,\s*id\)\s*<\s*\(\?\s*,\s*\?\)", detail)
        for detail in plans["video_page_after"]
    ):
        raise ConformanceFailure("cursor page did not use a tuple range seek")
    if not any(
        "video_uploads_org_state_idx" in detail
        for detail in plans["organization_snapshot"]
    ):
        raise ConformanceFailure("snapshot upload count did not use its named index")
    if not any(
        "media_jobs_org_state_idx" in detail
        for detail in plans["organization_snapshot"]
    ):
        raise ConformanceFailure("snapshot media-job count did not use its named index")
    snapshot_video_pk_joins = [
        detail
        for detail in plans["organization_snapshot"]
        if "sqlite_autoindex_videos_1" in detail
        and re.search(r"\bid\s*=\s*\?", detail)
    ]
    if len(snapshot_video_pk_joins) < 2:
        raise ConformanceFailure("snapshot child counts did not retain both video PK joins")
    return plans


def reserve_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


class WorkerServer:
    def __init__(self, d1: WranglerD1, token: str, root: pathlib.Path) -> None:
        self.d1 = d1
        self.token = token
        self.port = reserve_port()
        self.log_path = root / "worker.log"
        self.process: subprocess.Popen[str] | None = None
        self.log_file: Any = None

    def start(self) -> None:
        self.log_file = self.log_path.open("w", encoding="utf-8")
        command = [
            *self.d1.command,
            "dev",
            "--local",
            "--persist-to",
            str(self.d1.state),
            "--config",
            str(CONFIG),
            "--ip",
            "127.0.0.1",
            "--port",
            str(self.port),
            "--var",
            f"FRAME_REPOSITORY_CONFORMANCE_TOKEN:{self.token}",
        ]
        self.process = subprocess.Popen(
            command,
            cwd=ROOT,
            env=self.d1.environment,
            stdin=subprocess.DEVNULL,
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
            text=True,
        )
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise ConformanceFailure("local Worker exited before becoming ready")
            try:
                connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=1)
                connection.request("GET", "/health")
                response = connection.getresponse()
                response.read()
                connection.close()
                return
            except OSError:
                time.sleep(0.2)
        raise ConformanceFailure("local Worker did not become ready")

    def stop(self) -> None:
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        if self.log_file is not None:
            self.log_file.close()

    def request(
        self,
        scenario: str,
        *,
        token: str | None = None,
        path: str = CONFORMANCE_PATH,
        host: str | None = None,
        timeout: float = 30,
    ) -> tuple[int, dict[str, Any]]:
        body = json.dumps(
            {"schema_version": 1, "scenario": scenario},
            separators=(",", ":"),
        )
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
        headers = {
            "content-type": "application/json",
            TOKEN_HEADER: self.token if token is None else token,
        }
        if host is not None:
            headers["host"] = host
        connection.request("POST", path, body=body, headers=headers)
        response = connection.getresponse()
        raw = response.read()
        status = response.status
        connection.close()
        try:
            payload = json.loads(raw)
        except (json.JSONDecodeError, UnicodeDecodeError) as error:
            raise ConformanceFailure("local Worker returned invalid JSON") from error
        if not isinstance(payload, dict):
            raise ConformanceFailure("local Worker response shape changed")
        return status, payload

    def safe_telemetry_tail(self) -> str:
        if self.log_file is not None:
            self.log_file.flush()
        if not self.log_path.exists():
            return "none"
        pairs: list[str] = []
        for line in ANSI.sub("", self.log_path.read_text(encoding="utf-8")).splitlines():
            if '"event":"d1_repository_query"' not in line:
                continue
            match = re.search(
                r'"query_class":"([a-z_]+)".*"outcome":"([a-z_]+)"', line
            )
            if match:
                pairs.append(f"{match.group(1)}/{match.group(2)}")
        return ",".join(pairs[-4:]) or "none"


def expect_scenario(
    server: WorkerServer,
    scenario: str,
    status: int,
    outcome: str,
) -> dict[str, Any]:
    actual_status, payload = server.request(scenario)
    if actual_status != status or payload.get("outcome") != outcome:
        actual_outcome = payload.get("outcome")
        if not isinstance(actual_outcome, str) or not re.fullmatch(r"[a-z_]+", actual_outcome):
            actual_outcome = "invalid_outcome"
        raise ConformanceFailure(
            "compiled repository scenario returned an unexpected outcome: "
            f"{scenario}:{actual_status}:{actual_outcome}:tail={server.safe_telemetry_tail()}"
        )
    details = payload.get("details")
    if not isinstance(details, dict) or details.get("scenario") != scenario:
        raise ConformanceFailure("compiled repository scenario response was incomplete")
    values = details.get("values")
    if not isinstance(values, dict):
        raise ConformanceFailure("compiled repository scenario values were incomplete")
    return values


def exercise_worker(server: WorkerServer) -> None:
    wrong_token = secrets.token_hex(32)
    status, _ = server.request("reads_found", token=wrong_token)
    if status != 404:
        raise ConformanceFailure("local conformance token did not fail closed")
    status, _ = server.request("reads_found", path=CONFORMANCE_PATH + "/")
    if status != 404:
        raise ConformanceFailure("conformance path was not exact")
    status, _ = server.request(
        "reads_found", host=f"localhost:{server.port}"
    )
    if status != 404:
        raise ConformanceFailure("conformance path accepted a non-exact loopback host")

    found = expect_scenario(server, "reads_found", 200, "ok")
    expected_limit_one = [fixture_video_id(index) for index in (204, 203, 202, 201)]
    expected_limit_one_timestamps = [
        1_700_001_000_068,
        1_700_001_000_067,
        1_700_001_000_067,
        1_700_001_000_067,
    ]
    limit_one = found.get("limit_one", {})
    if (
        found.get("actor_can_update") is not True
        or found.get("video", {}).get("video_id") != VIDEO_A
        or found.get("upload", {}).get("upload_id") != UPLOAD_A
        or found.get("media_job", {}).get("job_id") != JOB_A
        or found.get("worker_job", {}).get("job_id") != JOB_A
        or found.get("organization", {}).get("id") != ORG_A
        or found.get("page", {}).get("count") != 207
        or found.get("page", {}).get("unique_count") != 207
        or found.get("bulk_boundary_count") != 205
        or [item.get("id") for item in found.get("bulk", [])] != [VIDEO_A]
        or limit_one.get("first_count") != 1
        or limit_one.get("first_has_next") is not True
        or limit_one.get("first_id") != expected_limit_one[0]
        or limit_one.get("second_count") != 1
        or limit_one.get("second_id") != expected_limit_one[1]
        or limit_one.get("first_id") == limit_one.get("second_id")
        or limit_one.get("sequence") != expected_limit_one
        or limit_one.get("timestamps") != expected_limit_one_timestamps
    ):
        raise ConformanceFailure("typed public read values changed")
    for scenario, outcome in (
        ("reads_not_found", "not_found"),
        ("invalid_input", "repository_invalid_request"),
        ("cross_tenant", "tenant_isolated"),
        ("corrupt_rows", "repository_corrupt_result"),
        ("denormalized_rows", "denormalized_rows_hidden"),
    ):
        values = expect_scenario(server, scenario, 200, outcome)
        if scenario in {"reads_not_found", "invalid_input", "cross_tenant"} and values.get(
            "method_count"
        ) != 7:
            raise ConformanceFailure("not every read method was exercised")
        if scenario == "invalid_input" and values.get("padded_title_rejected") is not True:
            raise ConformanceFailure("padded write titles did not fail at the repository boundary")
        if scenario == "corrupt_rows" and values != {"restored": True, "row_types": 7}:
            raise ConformanceFailure("persisted corrupt-row coverage or restoration changed")
        if scenario == "denormalized_rows" and values != {
            "restored": True,
            "row_types": 3,
            "tenant_views": 2,
            "snapshots": {
                "organization_a": {
                    "active_members": 1,
                    "active_videos": 207,
                    "active_uploads": 0,
                    "active_media_jobs": 0,
                },
                "organization_b": {
                    "active_members": 1,
                    "active_videos": 1,
                    "active_uploads": 0,
                    "active_media_jobs": 0,
                },
            },
        }:
            raise ConformanceFailure("denormalized child snapshot isolation changed")

    deadline = expect_scenario(server, "deadline", 503, "repository_timeout")
    if deadline != {"deadline_ms": 0, "query_dispatched": False}:
        raise ConformanceFailure("expired adapter deadline semantics changed")

    applied = expect_scenario(server, "write_success", 200, "applied")
    replay = expect_scenario(server, "write_replay", 200, "replay")
    batch_meta_changes = applied.pop("batch_meta_changes", None)
    if batch_meta_changes != 4 or applied != replay or applied.get("revision") != 5:
        raise ConformanceFailure(
            "ignored-envelope D1 batch metadata or stable replay changed: "
            f"changes={batch_meta_changes}"
        )
    for scenario in (
        "write_same_key_different_payload",
        "write_stale",
        "write_cross_tenant",
    ):
        expect_scenario(server, scenario, 409, "repository_conflict")
    rollback = expect_scenario(
        server, "write_constraint_rollback", 503, "repository_unavailable"
    )
    if rollback.get("rollback_expected") is not True:
        raise ConformanceFailure("constraint rollback was not explicit")

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
        futures = [
            executor.submit(server.request, "write_contention_left", timeout=30),
            executor.submit(server.request, "write_contention_right", timeout=30),
        ]
        outcomes = [future.result(timeout=35) for future in futures]
    statuses = sorted(status for status, _ in outcomes)
    result_codes = sorted(str(payload.get("outcome")) for _, payload in outcomes)
    if statuses != [200, 409] or result_codes != ["applied", "repository_conflict"]:
        raise ConformanceFailure("concurrent HTTP writes did not yield one winner and one conflict")

    expect_scenario(server, "unavailable", 503, "repository_unavailable")


def assert_final_state(d1: WranglerD1) -> None:
    video = d1.query(
        f"SELECT title,revision FROM videos WHERE id={sql_literal(VIDEO_A)}"
    )
    contention = d1.query(
        f"SELECT title,revision FROM videos WHERE id={sql_literal(CONTENTION_VIDEO)}"
    )
    commands = d1.query(
        "SELECT organization_id,idempotency_key,response_status,response_json "
        "FROM command_idempotency WHERE idempotency_key LIKE 'repository-%' "
        "ORDER BY idempotency_key"
    )
    outbox = d1.query(
        "SELECT aggregate_id,deduplication_key,event_type FROM outbox_events "
        "WHERE deduplication_key LIKE 'repository-video-title:%' "
        "ORDER BY deduplication_key"
    )
    guards = d1.query(
        "SELECT COUNT(*) AS count FROM repository_video_title_operations"
    )
    rejected_constraint = d1.query(
        "SELECT "
        "(SELECT COUNT(*) FROM command_idempotency "
        "WHERE idempotency_key='repository-constraint-0001') AS command_count,"
        "(SELECT COUNT(*) FROM outbox_events "
        "WHERE deduplication_key LIKE '%:repository-constraint-0001') AS outbox_count"
    )
    restored = d1.query(
        "SELECT "
        f"(SELECT organization_id FROM video_uploads WHERE id={sql_literal(UPLOAD_A)}) AS upload_org,"
        f"(SELECT organization_id FROM video_uploads WHERE id={sql_literal(UPLOAD_B)}) AS upload_b_org,"
        f"(SELECT content_type FROM video_uploads WHERE id={sql_literal(UPLOAD_A)}) AS content_type,"
        f"(SELECT organization_id FROM media_jobs WHERE id={sql_literal(JOB_A)}) AS job_org,"
        f"(SELECT organization_id FROM media_jobs WHERE id={sql_literal(JOB_B)}) AS job_b_org,"
        f"(SELECT attempt FROM media_jobs WHERE id={sql_literal(JOB_A)}) AS job_attempt,"
        f"(SELECT payload_json FROM media_jobs WHERE id={sql_literal(JOB_A)}) AS payload_json,"
        f"(SELECT name FROM organizations WHERE id={sql_literal(ORG_A)}) AS organization_name,"
        f"(SELECT title FROM videos WHERE id={sql_literal(fixture_video_id(204))}) AS page_title"
    )
    keys = {str(row["idempotency_key"]) for row in commands}
    contention_keys = {key for key in keys if key.startswith("repository-contention-")}
    if (
        video != [{"revision": 5, "title": "Repository \"Applied\" – O'Brien"}]
        or len(contention) != 1
        or contention[0].get("revision") != 11
        or contention[0].get("title") not in {"Contention Left", "Contention Right"}
        or keys != {"repository-success-0001", *contention_keys}
        or len(contention_keys) != 1
        or len(commands) != 2
        or any(row.get("response_status") != 200 for row in commands)
        or len(outbox) != 2
        or any(row.get("event_type") != "video.title_updated.v1" for row in outbox)
        or guards != [{"count": 0}]
        or rejected_constraint != [{"command_count": 0, "outbox_count": 0}]
        or restored
        != [
            {
                "content_type": "video/webm",
                "job_attempt": 1,
                "job_b_org": ORG_B,
                "job_org": ORG_A,
                "organization_name": "Tenant A",
                "page_title": "Page 204",
                "payload_json": '{"profile":"thumbnail_v1"}',
                "upload_b_org": ORG_B,
                "upload_org": ORG_A,
            }
        ]
    ):
        raise ConformanceFailure("atomic write or fixture-restoration invariant changed")


def parse_telemetry(log_path: pathlib.Path, token: str) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    decoder = json.JSONDecoder()
    for line in ANSI.sub("", log_path.read_text(encoding="utf-8")).splitlines():
        marker = '"event":"d1_repository_query"'
        if marker not in line:
            continue
        start = line.rfind("{", 0, line.index(marker) + 1)
        if start < 0:
            continue
        try:
            record, _ = decoder.raw_decode(line[start:])
        except json.JSONDecodeError:
            continue
        if isinstance(record, dict) and record.get("event") == "d1_repository_query":
            records.append(record)
    if not records:
        raise ConformanceFailure("Worker emitted no repository telemetry")
    fields = {
        "event",
        "query_class",
        "duration_ms",
        "rows",
        "retries",
        "bookmark_use",
        "outcome",
    }
    forbidden = (ORG_A, ORG_B, VIDEO_A, VIDEO_B, UPLOAD_A, JOB_A, token, "SELECT", "UPDATE")
    for record in records:
        encoded = json.dumps(record, sort_keys=True, separators=(",", ":"))
        if set(record) != fields or any(value in encoded for value in forbidden):
            raise ConformanceFailure("Worker telemetry fields or redaction changed")
        if (
            not isinstance(record.get("duration_ms"), int)
            or int(record["duration_ms"]) < 0
            or not isinstance(record.get("rows"), int)
            or int(record["rows"]) < 0
            or record.get("retries") != 0
            or record.get("bookmark_use") != "unavailable_in_workers_binding"
        ):
            raise ConformanceFailure("Worker telemetry value bounds changed")
    outcomes = {str(record["outcome"]) for record in records}
    required = {
        "ok",
        "repository_conflict",
        "repository_timeout",
        "repository_unavailable",
        "repository_corrupt_result",
    }
    if not required.issubset(outcomes):
        raise ConformanceFailure("Worker telemetry did not cover every mapped runtime outcome")
    pairs = {
        (str(record["query_class"]), str(record["outcome"])) for record in records
    }
    required_corrupt_pairs = {
        ("video_page", "repository_corrupt_result"),
        ("video_bulk", "repository_corrupt_result"),
        ("media_job_aggregate", "repository_corrupt_result"),
        ("native_worker_job_aggregate", "repository_corrupt_result"),
    }
    if not required_corrupt_pairs.issubset(pairs):
        raise ConformanceFailure("semantic corrupt-row telemetry coverage changed")
    return records


def artifact_digest(files: Sequence[pathlib.Path]) -> str:
    digest = hashlib.sha256()
    for path in files:
        digest.update(path.name.encode())
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def telemetry_samples(records: Sequence[dict[str, Any]]) -> list[dict[str, Any]]:
    samples: dict[tuple[str, str], dict[str, Any]] = {}
    for record in records:
        key = (str(record["query_class"]), str(record["outcome"]))
        samples.setdefault(key, record)
    return [samples[key] for key in sorted(samples)]


def write_evidence(
    path: pathlib.Path,
    plans: dict[str, list[str]],
    telemetry: Sequence[dict[str, Any]],
) -> None:
    migrations = migration_files()
    queries = sorted(QUERIES.glob("*.sql"))
    report = {
        "schema_version": 1,
        "suite": "frame-d1-aggregate-repository-conformance",
        "runtime_boundary": "compiled_rust_wasm_worker_over_loopback_http",
        "database": "isolated_local_wrangler_d1",
        "wrangler_version": WRANGLER_VERSION,
        "migration_count": len(migrations),
        "migration_digest_sha256": artifact_digest(migrations),
        "query_count": len(queries),
        "query_digest_sha256": artifact_digest(queries),
        "fixture": {"tenants": 2, "videos": 208, "page_rows": 205},
        "scenarios": [
            "seven_reads_found_not_found_invalid_cross_tenant",
            "deep_keyset_pagination",
            "nonempty_limit_one_tied_timestamp_pagination",
            "bulk_parameter_chunking",
            "persisted_row_corruption_and_u32_attempt_bounds",
            "denormalized_org_video_rows_and_snapshot_counts",
            "expired_adapter_deadline",
            "provider_schema_unavailable",
            "optimistic_idempotent_title_write",
            "stable_replay_and_payload_conflict",
            "stale_and_cross_tenant_conflict",
            "constraint_rollback",
            "concurrent_http_compare_and_swap",
            "query_plan",
            "actual_worker_telemetry",
        ],
        "result": "pass",
        "query_plans": plans,
        "telemetry_record_count": len(telemetry),
        "telemetry_samples": telemetry_samples(telemetry),
        "not_claimed": [
            "production_d1_replication_lag",
            "workers_binding_session_bookmarks",
            "provider_network_timeout",
            "remote_d1_capacity_or_contention",
        ],
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wrangler-bin", help="direct path to pinned Wrangler 4.111.0")
    parser.add_argument(
        "--evidence",
        type=pathlib.Path,
        default=ROOT / "target" / "evidence" / "d1-repository-conformance.json",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(sys.argv[1:] if argv is None else argv)
    reference: sqlite3.Connection | None = None
    try:
        refuse_external_authority()
        verify_rust_contract()
        wrangler = detect_wrangler(arguments.wrangler_bin)
        reference = apply_reference()
        assert_reference_write_operation(reference)
        with tempfile.TemporaryDirectory(prefix="frame-d1-conformance-") as directory:
            root = pathlib.Path(directory)
            state = root / "state"
            state.mkdir(mode=0o700)
            d1 = WranglerD1(wrangler, state)
            d1.migrate()
            fixture = root / "fixture.sql"
            write_fixture(fixture)
            d1.execute_file(fixture)
            assert_reference_query_parity(reference, d1)
            plans = query_plans(d1)

            token = secrets.token_hex(32)
            server = WorkerServer(d1, token, root)
            try:
                server.start()
                exercise_worker(server)
            finally:
                server.stop()
            assert_final_state(d1)
            telemetry = parse_telemetry(server.log_path, token)
        write_evidence(arguments.evidence.resolve(), plans, telemetry)
    except (
        ConformanceFailure,
        OSError,
        sqlite3.Error,
        subprocess.SubprocessError,
        ValueError,
    ) as error:
        print(f"D1 repository conformance failed: {error}", file=sys.stderr)
        return 1
    finally:
        if reference is not None:
            reference.close()
    print(
        "D1 aggregate repository conformance passed through compiled Worker "
        f"({len(migration_files())} migrations; Wrangler {WRANGLER_VERSION})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
