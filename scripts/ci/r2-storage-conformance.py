#!/usr/bin/env python3
"""Exercise ObjectStoreV1 through Wrangler's credential-free local R2 binding."""

from __future__ import annotations

import argparse
import hashlib
import http.client
import importlib.util
import json
import os
import pathlib
import re
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable, Sequence
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
RUNNER_SOURCE = pathlib.Path(__file__).resolve()
CONFIG = ROOT / "apps" / "control-plane" / "wrangler.local.toml"
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
CONTRACT_MIGRATIONS = ROOT / "apps" / "control-plane" / "contract-migrations"
DIRECT_SQLITE_CONFORMANCE = ROOT / "scripts" / "ci" / "direct-upload-sqlite-conformance.py"
COMPLETION_RECONCILIATION_SQLITE = (
    ROOT / "scripts" / "ci" / "r2-completion-reconciliation-sqlite-conformance.py"
)
MULTIPART_ENFORCEMENT = CONTRACT_MIGRATIONS / "0033_r2_multipart_claims_enforce.sql"
R2_SOURCE = ROOT / "apps" / "control-plane" / "src" / "r2_storage.rs"
MULTIPART_SOURCE = ROOT / "apps" / "control-plane" / "src" / "r2_multipart.rs"
ROUTING_SOURCE = ROOT / "apps" / "control-plane" / "src" / "routing.rs"
LIB_SOURCE = ROOT / "apps" / "control-plane" / "src" / "lib.rs"
CONFORMANCE_PATH = "/__frame/local/r2-storage-conformance"
DATABASE = "frame-local"
WRANGLER_VERSION = "4.111.0"
ANSI = re.compile(r"\x1b\[[0-9;]*m")


class ConformanceFailure(RuntimeError):
    """A stable assertion that never exposes provider output or local paths."""


def refuse_external_authority() -> None:
    forbidden = [
        name
        for name in (
            "CLOUDFLARE_API_TOKEN",
            "CLOUDFLARE_ACCOUNT_ID",
            "CLOUDFLARE_API_KEY",
            "CLOUDFLARE_EMAIL",
            "DATABASE_URL",
        )
        if os.environ.get(name)
    ]
    if os.environ.get("FRAME_DEPLOYMENT") == "production":
        forbidden.append("FRAME_DEPLOYMENT")
    if forbidden:
        raise ConformanceFailure("local R2 conformance refused external authority variables")


def detect_wrangler(explicit: str | None) -> list[str]:
    command = (
        (["node", explicit] if explicit and explicit.endswith(".js") else [explicit])
        if explicit
        else ["npx", "--yes", f"wrangler@{WRANGLER_VERSION}"]
    )
    environment = os.environ.copy()
    environment.update(
        {
            "NO_COLOR": "1",
            "WRANGLER_LOG_PATH": "/tmp/frame-r2-wrangler-version.log",
            "WRANGLER_SEND_METRICS": "false",
        }
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


def verify_checked_in_surface() -> None:
    config = CONFIG.read_text(encoding="utf-8")
    source = R2_SOURCE.read_text(encoding="utf-8")
    multipart = MULTIPART_SOURCE.read_text(encoding="utf-8")
    routing = ROUTING_SOURCE.read_text(encoding="utf-8")
    library = LIB_SOURCE.read_text(encoding="utf-8")
    for marker in (
        'binding = "RECORDINGS"',
        'bucket_name = "frame-recordings-local"',
        'FRAME_DEPLOYMENT = "local"',
    ):
        if marker not in config:
            raise ConformanceFailure("local R2 binding configuration drifted")
    for marker in (
        "impl ObjectStoreV1 for R2ObjectStoreV1",
        "Conditional {",
        "etag_does_not_match: Some(\"*\".into())",
        "StorageFailureKind::NotFound",
        "cross_tenant_not_found",
        ".without(ObjectStoreOperation::ConditionalDeleteVersion)",
        "delete/recreate race",
        "run_local_contract(&adapter)",
    ):
        if marker not in source:
            raise ConformanceFailure("R2 adapter contract surface drifted")
    for marker in (
        "pub async fn run_local_contract(",
        "R2MultipartObjectStoreV1::new(bucket, database, &probe)",
        "r2_multipart_creation_claims_v1",
        "reserve_provider_creation(",
        "reconcile_provider_creation(",
        "r2_multipart_completion_claims_v1",
        "claim_completion(",
        "r2_multipart_completion_reconciliation_v1",
        "quarantine_exhausted_completion_reconciliation(",
        "completion_state_admissible(",
        ".complete_multipart(context, completion_request.clone())",
        "cleanup_stale(",
    ):
        if marker not in multipart:
            raise ConformanceFailure("R2 multipart contract surface drifted")
    expansion = (MIGRATIONS / "0028_r2_multipart_part_claims.sql").read_text(
        encoding="utf-8"
    )
    enforcement = MULTIPART_ENFORCEMENT.read_text(encoding="utf-8")
    for marker in (
        "r2_multipart_claim_rollout_v1",
        "phase TEXT NOT NULL CHECK (phase IN ('fenced', 'enabled'))",
        "r2_multipart_creation_claims_v1",
        "r2_multipart_completion_claims_v1",
        "r2_multipart_part_claims_v1",
    ):
        if marker not in expansion:
            raise ConformanceFailure("R2 multipart expansion surface drifted")
    for marker in (
        "frame_r2_multipart_claim_contract_not_drained_v1",
        "legacy_provider_mutations_drained",
        "LEFT JOIN r2_multipart_parts_v1 part",
        "SELECT 1 FROM r2_multipart_completion_reconciliation_v1",
        "r2_multipart_sessions_v1_creation_authority",
        "r2_multipart_abort_reconciliation_v1_completion_exclusion",
        "NEW.completion_claim_token IS NULL",
    ):
        if marker in expansion or marker not in enforcement:
            raise ConformanceFailure("R2 multipart contract-phase fence drifted")
    for marker in (
        "require_provider_mutations_enabled().await?",
        "if !self.provider_mutations_enabled().await?",
    ):
        if marker not in multipart:
            raise ConformanceFailure("R2 multipart runtime rollout fence drifted")
    if (
        f'path == "{CONFORMANCE_PATH}"' not in routing
        or "Route::LocalR2StorageConformance" not in routing
        or "config.production() || !valid_repository_conformance_target" not in library
    ):
        raise ConformanceFailure("loopback-only R2 conformance route drifted")


def expect_sqlite_integrity(
    operation: Callable[[], object],
    fragment: str,
) -> None:
    try:
        operation()
    except sqlite3.IntegrityError as error:
        if fragment not in str(error):
            raise ConformanceFailure(
                "R2 SQLite fence returned the wrong authority error"
            ) from error
    else:
        raise ConformanceFailure("R2 SQLite fence accepted a mixed-version mutation")


def load_direct_sqlite_module() -> Any:
    specification = importlib.util.spec_from_file_location(
        "frame_direct_upload_conformance_for_r2", DIRECT_SQLITE_CONFORMANCE
    )
    if specification is None or specification.loader is None:
        raise ConformanceFailure("direct-upload SQLite fixture is unavailable")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


def verify_mixed_version_sql_fences() -> None:
    direct = load_direct_sqlite_module()
    database = sqlite3.connect(":memory:")
    try:
        direct.migrate(database)
        direct.seed(database)
        expansions = [
            path
            for path in sorted(MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql"))
            if 24 <= int(path.name[:4]) <= 31
        ]
        expected_expansions = list(range(24, 32))
        if [int(path.name[:4]) for path in expansions] != expected_expansions:
            raise ConformanceFailure("R2 SQLite migration sequence is not contiguous")
        # Build released 0024 rows before installing the later authority
        # triggers. The assertions below then model an old Worker continuing
        # against a database at the R2 expansion boundary through 0031,
        # immediately before the separately protected 0033 contract phase.
        database.executescript(expansions[0].read_text(encoding="utf-8"))

        now = int(direct.NOW)
        organization_id = str(direct.ORG)
        owner_id = str(direct.USER)
        video_id = str(direct.VIDEO)
        expected_bytes = 10 * 1024 * 1024
        part_size = 5 * 1024 * 1024
        checksum = "ab" * 32
        completion_digest = "cd" * 32
        expiry = now + 600_000
        capabilities = json.dumps(
            {"multipart": True, "schema_version": 1},
            sort_keys=True,
            separators=(",", ":"),
        )
        integration_id = "018f47a6-7b1c-7f55-8f39-8f8a8690f701"
        database.execute(
            "INSERT INTO storage_integrations(id,organization_id,owner_user_id,provider,state,"
            "capabilities_json,credential_ciphertext,created_at_ms,updated_at_ms,revision,"
            "capabilities_schema_version,capabilities_checksum) "
            "VALUES (?,?,?,'r2','active',?,'fixture-ciphertext',?,?,0,1,?)",
            (
                integration_id,
                organization_id,
                owner_id,
                capabilities,
                now - 100,
                now - 100,
                hashlib.sha256(capabilities.encode()).hexdigest(),
            ),
        )

        def seed_upload(upload_id: str, suffix: str) -> str:
            object_key = (
                f"tenants/{organization_id}/videos/{video_id}/source/v2/{suffix}.mp4"
            )
            database.execute(
                "INSERT INTO video_uploads(id,organization_id,video_id,state,expected_bytes,"
                "received_bytes,idempotency_key,source_object_key,source_version,content_type,"
                "checksum_sha256,created_at_ms,updated_at_ms,revision,event_sequence,"
                "event_fingerprint,transfer_mode,direct_staging_key,direct_checksum_sha256,"
                "direct_expires_at_ms) VALUES (?,?,?,'initiated',?,0,?,?,2,'video/mp4',NULL,"
                "?,?,0,0,?,'brokered',NULL,NULL,NULL)",
                (
                    upload_id,
                    organization_id,
                    video_id,
                    expected_bytes,
                    f"r2-mixed-version-{suffix}",
                    object_key,
                    now,
                    now,
                    "daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9",
                ),
            )
            database.execute(
                "INSERT INTO r2_multipart_intents_v1(upload_id,integration_id,checksum_sha256,"
                "part_size,part_count,expires_at_ms,created_at_ms) VALUES (?,?,?,?,2,?,?)",
                (upload_id, integration_id, checksum, part_size, expiry, now),
            )
            return object_key

        session_upload = "018f47a6-7b1c-7f55-8f39-8f8a8690d701"
        completion_upload = "018f47a6-7b1c-7f55-8f39-8f8a8690d702"
        abort_upload = "018f47a6-7b1c-7f55-8f39-8f8a8690d703"
        legacy_upload = "018f47a6-7b1c-7f55-8f39-8f8a8690d704"
        session_key = seed_upload(session_upload, "session-authority")
        completion_key = seed_upload(completion_upload, "completion-first")
        abort_key = seed_upload(abort_upload, "abort-first")
        legacy_key = seed_upload(legacy_upload, "n-minus-one-expand")
        for path in expansions[1:]:
            database.executescript(path.read_text(encoding="utf-8"))

        def reserve(upload_id: str, object_key: str, claim_token: str) -> None:
            database.execute(
                "INSERT INTO r2_multipart_creation_claims_v1(upload_id,organization_id,"
                "object_key,expected_bytes,checksum_sha256,content_type,correlation_id,"
                "part_size,part_count,expires_at_ms,claim_token,state,provider_upload_id,"
                "created_at_ms,updated_at_ms) VALUES (?,?,?,?,?,'video/mp4',?,?,2,?,?,"
                "'reserved',NULL,?,?)",
                (
                    upload_id,
                    organization_id,
                    object_key,
                    expected_bytes,
                    checksum,
                    upload_id,
                    part_size,
                    expiry,
                    claim_token,
                    now,
                    now,
                ),
            )

        def insert_session(upload_id: str, object_key: str, provider_upload_id: str) -> None:
            database.execute(
                "INSERT INTO r2_multipart_sessions_v1(upload_id,object_key,provider_upload_id,"
                "state,expected_bytes,checksum_sha256,content_type,correlation_id,created_at_ms,"
                "expires_at_ms,completed_at_ms) VALUES (?,?,?,'open',?,?,'video/mp4',?,?,?,NULL)",
                (
                    upload_id,
                    object_key,
                    provider_upload_id,
                    expected_bytes,
                    checksum,
                    upload_id,
                    now,
                    expiry,
                ),
            )

        def bind_and_insert_session(
            upload_id: str,
            object_key: str,
            claim_token: str,
            provider_upload_id: str,
        ) -> None:
            reserve(upload_id, object_key, claim_token)
            database.execute(
                "UPDATE r2_multipart_creation_claims_v1 SET state='provider_bound',"
                "provider_upload_id=?,updated_at_ms=? WHERE upload_id=?",
                (provider_upload_id, now + 1, upload_id),
            )
            insert_session(upload_id, object_key, provider_upload_id)
            database.execute(
                "UPDATE r2_multipart_creation_claims_v1 SET state='committed',updated_at_ms=? "
                "WHERE upload_id=?",
                (now + 2, upload_id),
            )

        # Migration 0028 is an expand phase. A released N-1 Worker must still
        # be able to write its nullable-token session, part, and completion
        # shapes until the separately protected contract gate is approved.
        insert_session(legacy_upload, legacy_key, "provider-n-minus-one")
        database.execute(
            "INSERT INTO r2_multipart_parts_v1(upload_id,part_number,bytes,"
            "checksum_sha256,provider_etag,uploaded_at_ms) VALUES (?,1,?,?,?,?)",
            (legacy_upload, part_size, checksum, "legacy-etag", now + 1),
        )
        database.execute(
            "INSERT INTO r2_multipart_completions_v1(upload_id,request_parts_sha256,"
            "provider_version,provider_etag,bytes,checksum_sha256,content_type,container,"
            "video_codec,audio_codec,width,height,duration_ms,frame_rate_millihertz,"
            "completed_at_ms,correlation_id) VALUES (?,?,?,'legacy-etag',?,?,'video/mp4',"
            "'mp4','h264','aac',1920,1080,60000,30000,?,?)",
            (
                legacy_upload,
                completion_digest,
                "legacy-version",
                expected_bytes,
                checksum,
                now + 2,
                legacy_upload,
            ),
        )
        if database.execute(
            "SELECT (SELECT COUNT(*) FROM r2_multipart_sessions_v1 WHERE upload_id=?),"
            "(SELECT COUNT(*) FROM r2_multipart_parts_v1 WHERE upload_id=? "
            "AND part_claim_token IS NULL),"
            "(SELECT COUNT(*) FROM r2_multipart_completions_v1 WHERE upload_id=? "
            "AND completion_claim_token IS NULL)",
            (legacy_upload, legacy_upload, legacy_upload),
        ).fetchone() != (1, 1, 1):
            raise ConformanceFailure("0028 expansion rejected an N-1 multipart write")

        if database.execute(
            "SELECT phase FROM r2_multipart_claim_rollout_v1 WHERE singleton=1"
        ).fetchone() != ("fenced",):
            raise ConformanceFailure("0028 did not fence the claim-aware Worker by default")

        enforcement_sql = MULTIPART_ENFORCEMENT.read_text(encoding="utf-8")
        try:
            database.executescript(f"BEGIN IMMEDIATE;\n{enforcement_sql}\nCOMMIT;")
        except sqlite3.IntegrityError as error:
            if "frame_r2_multipart_claim_contract_not_drained_v1" not in str(error):
                raise ConformanceFailure(
                    "0033 returned the wrong pre-drain contract failure"
                ) from error
            database.execute("ROLLBACK")
        else:
            raise ConformanceFailure("0033 enabled provider mutations before legacy drain")
        if database.execute(
            "SELECT state FROM r2_multipart_sessions_v1 WHERE upload_id=?",
            (legacy_upload,),
        ).fetchone() != ("open",):
            raise ConformanceFailure("refused 0033 contract mutated the legacy session")

        # Model the protected gate waiting for the N-1 provider request to
        # finish. The expansion still accepts this terminal transition; only
        # after zero nonterminal sessions remain may enforcement be enabled.
        database.execute(
            "UPDATE r2_multipart_sessions_v1 SET state='complete',completed_at_ms=? "
            "WHERE upload_id=? AND state='open'",
            (now + 2, legacy_upload),
        )

        database.executescript(enforcement_sql)
        if database.execute(
            "SELECT phase FROM r2_multipart_claim_rollout_v1 WHERE singleton=1"
        ).fetchone() != ("enabled",):
            raise ConformanceFailure("0033 did not release the claim-aware Worker")
        if database.execute(
            "SELECT assertion FROM r2_multipart_claim_contract_assertions_v1 WHERE singleton=1"
        ).fetchone() != ("legacy_provider_mutations_drained",):
            raise ConformanceFailure("0033 did not retain its immutable drain assertion")

        expect_sqlite_integrity(
            lambda: insert_session(session_upload, session_key, "provider-session-authority"),
            "frame_r2_multipart_session_creation_authority_v1",
        )
        session_claim = "018f47a6-7b1c-7f55-8f39-8f8a8690e701"
        reserve(session_upload, session_key, session_claim)
        expect_sqlite_integrity(
            lambda: insert_session(session_upload, session_key, "provider-session-authority"),
            "frame_r2_multipart_session_creation_authority_v1",
        )
        database.execute(
            "UPDATE r2_multipart_creation_claims_v1 SET state='provider_bound',"
            "provider_upload_id='provider-session-authority',updated_at_ms=? WHERE upload_id=?",
            (now + 1, session_upload),
        )
        expect_sqlite_integrity(
            lambda: insert_session(session_upload, session_key, "wrong-provider-handle"),
            "frame_r2_multipart_session_creation_authority_v1",
        )
        insert_session(session_upload, session_key, "provider-session-authority")
        expect_sqlite_integrity(
            lambda: database.execute(
                "UPDATE r2_multipart_sessions_v1 SET state='completing' WHERE upload_id=?",
                (session_upload,),
            ),
            "frame_r2_multipart_session_transition_v1",
        )
        session_completion_claim = "018f47a6-7b1c-7f55-8f39-8f8a8690e711"
        database.execute(
            "INSERT INTO r2_multipart_completion_claims_v1(upload_id,request_parts_sha256,"
            "claim_token,state,attempt_count,claimed_at_ms,lease_expires_at_ms,completed_at_ms) "
            "VALUES (?,?,?,'active',1,?,?,NULL)",
            (
                session_upload,
                completion_digest,
                session_completion_claim,
                now + 5,
                now + 65_000,
            ),
        )
        expect_sqlite_integrity(
            lambda: database.execute(
                "UPDATE r2_multipart_sessions_v1 SET state='complete',completed_at_ms=? "
                "WHERE upload_id=?",
                (now + 10, session_upload),
            ),
            "frame_r2_multipart_session_transition_v1",
        )
        expect_sqlite_integrity(
            lambda: database.execute(
                "INSERT INTO r2_multipart_completions_v1(upload_id,request_parts_sha256,"
                "provider_version,provider_etag,bytes,checksum_sha256,content_type,container,"
                "video_codec,audio_codec,width,height,duration_ms,frame_rate_millihertz,"
                "completed_at_ms,correlation_id) VALUES (?,?,?,'etag',?,?,'video/mp4',"
                "'mp4','h264','aac',1920,1080,60000,30000,?,?)",
                (
                    session_upload,
                    completion_digest,
                    "provider-version",
                    expected_bytes,
                    checksum,
                    now + 10,
                    session_upload,
                ),
            ),
            "frame_r2_multipart_completion_authority_v1",
        )

        completion_claim = "018f47a6-7b1c-7f55-8f39-8f8a8690e702"
        bind_and_insert_session(
            completion_upload,
            completion_key,
            completion_claim,
            "provider-completion-first",
        )
        active_completion_claim = "018f47a6-7b1c-7f55-8f39-8f8a8690e712"
        database.execute(
            "INSERT INTO r2_multipart_completion_claims_v1(upload_id,request_parts_sha256,"
            "claim_token,state,attempt_count,claimed_at_ms,lease_expires_at_ms,completed_at_ms) "
            "VALUES (?,?,?,'active',1,?,?,NULL)",
            (
                completion_upload,
                completion_digest,
                active_completion_claim,
                now + 10,
                now + 70_000,
            ),
        )
        if database.execute(
            "SELECT state FROM r2_multipart_sessions_v1 WHERE upload_id=?",
            (completion_upload,),
        ).fetchone() != ("completing",):
            raise ConformanceFailure("completion claim did not linearize the SQLite session")
        expect_sqlite_integrity(
            lambda: database.execute(
                "INSERT INTO r2_multipart_abort_reconciliation_v1(upload_id,intent_kind,state,"
                "attempt_count,next_attempt_at_ms,last_failure_class,started_at_ms,updated_at_ms,"
                "terminal_at_ms) VALUES (?,'authenticated_delete','pending',1,?,NULL,?,?,NULL)",
                (completion_upload, now + 20, now + 20, now + 20),
            ),
            "frame_r2_multipart_completion_abort_exclusion_v1",
        )

        abort_creation_claim = "018f47a6-7b1c-7f55-8f39-8f8a8690e703"
        bind_and_insert_session(
            abort_upload,
            abort_key,
            abort_creation_claim,
            "provider-abort-first",
        )
        database.execute(
            "INSERT INTO r2_multipart_part_claims_v1(upload_id,part_number,bytes,"
            "checksum_sha256,claim_token,claimed_at_ms,lease_expires_at_ms) "
            "VALUES (?,1,?,?,?,?,?)",
            (
                abort_upload,
                part_size,
                checksum,
                "018f47a6-7b1c-7f55-8f39-8f8a8690e723",
                now + 10,
                now + 60_000,
            ),
        )
        expect_sqlite_integrity(
            lambda: database.execute(
                "UPDATE r2_multipart_part_claims_v1 SET claim_token=?,claimed_at_ms=?,"
                "lease_expires_at_ms=? WHERE upload_id=? AND part_number=1",
                (
                    "018f47a6-7b1c-7f55-8f39-8f8a8690e724",
                    now + 20,
                    now + 70_000,
                    abort_upload,
                ),
            ),
            "frame_r2_multipart_part_claim_v1",
        )
        database.execute(
            "UPDATE r2_multipart_part_claims_v1 SET claim_token=?,claimed_at_ms=?,"
            "lease_expires_at_ms=? WHERE upload_id=? AND part_number=1",
            (
                "018f47a6-7b1c-7f55-8f39-8f8a8690e725",
                now + 60_000,
                now + 120_000,
                abort_upload,
            ),
        )
        database.execute(
            "INSERT INTO r2_multipart_abort_reconciliation_v1(upload_id,intent_kind,state,"
            "attempt_count,next_attempt_at_ms,last_failure_class,started_at_ms,updated_at_ms,"
            "terminal_at_ms) VALUES (?,'authenticated_delete','pending',1,?,NULL,?,?,NULL)",
            (abort_upload, now + 20, now + 20, now + 20),
        )
        expect_sqlite_integrity(
            lambda: database.execute(
                "INSERT INTO r2_multipart_completion_claims_v1(upload_id,request_parts_sha256,"
                "claim_token,state,attempt_count,claimed_at_ms,lease_expires_at_ms,"
                "completed_at_ms) VALUES (?,?,?,'active',1,?,?,NULL)",
                (
                    abort_upload,
                    completion_digest,
                    "018f47a6-7b1c-7f55-8f39-8f8a8690e713",
                    now + 30,
                    now + 80_000,
                ),
            ),
            "frame_r2_multipart_completion_claim_v1",
        )
        database.rollback()
    finally:
        database.close()


def verify_completion_reconciliation_sqlite() -> None:
    result = subprocess.run(
        [sys.executable, "-I", str(COMPLETION_RECONCILIATION_SQLITE)],
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    if result.returncode != 0:
        raise ConformanceFailure("R2 completion reconciliation SQLite proof failed")
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise ConformanceFailure(
            "R2 completion reconciliation proof returned invalid evidence"
        ) from error
    if (
        payload.get("later_row_selected_after_quarantine") is not True
        or payload.get("expired_final_attempt_terminalized") is not True
        or payload.get("post_expand_n_minus_one_scheduler_eligible") is not True
        or payload.get("post_expand_n_minus_one_completion_terminalized") is not True
        or payload.get("matching_active_claim_not_stranded") is not True
        or payload.get("preexisting_completion_backfill_validated") is not True
    ):
        raise ConformanceFailure("R2 completion reconciliation evidence drifted")


def reserve_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


class WorkerServer:
    def __init__(self, command: list[str], root: pathlib.Path) -> None:
        self.command = command
        self.root = root
        self.state = root / "state"
        self.state.mkdir(mode=0o700)
        self.port = reserve_port()
        self.log_path = root / "worker.log"
        self.process: subprocess.Popen[str] | None = None
        self.log_file: Any = None
        self.environment = os.environ.copy()
        self.environment.update(
            {
                "CI": "true",
                "NO_COLOR": "1",
                "WRANGLER_LOG_PATH": str(root / "wrangler.log"),
                "WRANGLER_SEND_METRICS": "false",
            }
        )

    def start(self) -> None:
        migration = subprocess.run(
            [
                *self.command,
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
            cwd=ROOT,
            env=self.environment,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=180,
            check=False,
        )
        if migration.returncode != 0:
            raise ConformanceFailure("local R2 D1 migration apply failed")
        contract = subprocess.run(
            [
                *self.command,
                "d1",
                "execute",
                DATABASE,
                "--local",
                "--persist-to",
                str(self.state),
                "--config",
                str(CONFIG),
                "--file",
                str(MULTIPART_ENFORCEMENT),
            ],
            cwd=ROOT,
            env=self.environment,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=180,
            check=False,
        )
        if contract.returncode != 0:
            raise ConformanceFailure("local R2 contract migration apply failed")
        self.log_file = self.log_path.open("w", encoding="utf-8")
        self.process = subprocess.Popen(
            [
                *self.command,
                "dev",
                "--local",
                "--persist-to",
                str(self.state),
                "--config",
                str(CONFIG),
                "--ip",
                "127.0.0.1",
                "--port",
                str(self.port),
            ],
            cwd=ROOT,
            env=self.environment,
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
                status, _, _ = self.request("GET", "/health", timeout=1)
                if status == 200:
                    return
                raise ConformanceFailure("local Worker health response changed")
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
        method: str,
        path: str,
        *,
        host: str | None = None,
        timeout: float = 180,
    ) -> tuple[int, bytes, dict[str, str]]:
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
        headers = {"content-length": "0"}
        if host is not None:
            headers["host"] = host
        connection.request(method, path, headers=headers)
        response = connection.getresponse()
        raw = response.read()
        status = response.status
        response_headers = {key.lower(): value for key, value in response.getheaders()}
        connection.close()
        return status, raw, response_headers


def decode_contract(raw: bytes) -> dict[str, Any]:
    try:
        payload = json.loads(raw)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise ConformanceFailure("local R2 route returned invalid JSON") from error
    if not isinstance(payload, dict):
        raise ConformanceFailure("local R2 route response shape changed")
    return payload


def exercise_worker(server: WorkerServer) -> dict[str, Any]:
    status, raw, headers = server.request("POST", CONFORMANCE_PATH)
    if status != 200 or not headers.get("content-type", "").startswith("application/json"):
        raise ConformanceFailure("local R2 contract request failed")
    payload = decode_contract(raw)
    expected = {
        "schema_version": 1,
        "adapter": "cloudflare_r2_worker_binding_v1",
        "operations": ["put", "head", "get", "range", "copy", "delete", "list"],
        "multipart_operations": [
            "create",
            "lookup",
            "list_parts",
            "put_part",
            "complete",
            "abort",
            "stale_cleanup",
            "head",
            "range",
        ],
        "multipart_conditions": [
            "durable_pre_provider_create_claim",
            "provider_handle_reconciliation",
            "leased_part_write_claim",
            "strict_completion_linearization",
            "completion_abort_exclusion",
            "expired_open_completion_rejected",
        ],
        "conditions": [
            "immutable_create",
            "exact_replay",
            "conditional_source_version",
            "conditional_delete_unsupported",
            "cross_tenant_not_found",
        ],
        "status": "passed",
    }
    if payload != expected:
        raise ConformanceFailure("local R2 adapter report changed")

    replay_status, replay_raw, _ = server.request("POST", CONFORMANCE_PATH)
    if replay_status != 200 or decode_contract(replay_raw) != expected:
        raise ConformanceFailure("local R2 adapter contract is not replay-safe")
    if server.request("GET", CONFORMANCE_PATH)[0] != 405:
        raise ConformanceFailure("local R2 route method guard changed")
    for lookalike in (
        f"{CONFORMANCE_PATH}/",
        f"{CONFORMANCE_PATH}%2f",
        f"{CONFORMANCE_PATH}/objects",
    ):
        if server.request("POST", lookalike)[0] != 404:
            raise ConformanceFailure("local R2 route accepted a path lookalike")
    if server.request(
        "POST", CONFORMANCE_PATH, host=f"localhost:{server.port}"
    )[0] != 404:
        raise ConformanceFailure("local R2 route accepted a non-IPv4-loopback authority")
    return payload


def digest_sources() -> str:
    digest = hashlib.sha256()
    paths = (
        RUNNER_SOURCE,
        CONFIG,
        R2_SOURCE,
        MULTIPART_SOURCE,
        ROUTING_SOURCE,
        LIB_SOURCE,
        COMPLETION_RECONCILIATION_SQLITE,
        MULTIPART_ENFORCEMENT,
        *sorted(MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql")),
    )
    for path in paths:
        digest.update(path.relative_to(ROOT).as_posix().encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def write_evidence(path: pathlib.Path, payload: dict[str, Any]) -> None:
    report = {
        "schema_version": 1,
        "suite": "frame-r2-worker-binding-conformance",
        "runtime_boundary": "compiled_rust_wasm_worker_over_loopback_http",
        "storage": "isolated_wrangler_local_r2",
        "credential_mode": "none",
        "wrangler_version": WRANGLER_VERSION,
        "contract": payload,
        "scenarios": [
            "immutable_create_and_exact_replay",
            "head_get_and_bounded_range",
            "same_scope_conditional_copy",
            "provider_version_fenced_copy_and_fail_closed_delete",
            "scoped_cursor_pagination",
            "cross_tenant_not_found_for_every_operation",
            "idempotent_delete_and_contract_replay",
            "multipart_create_exact_replay_and_resume",
            "multipart_pre_provider_create_claim_and_handle_reconciliation",
            "multipart_concurrent_conflicting_part_claim",
            "multipart_part_receipt_replay_and_contiguous_completion",
            "multipart_full_object_sha256_and_exact_complete_replay",
            "multipart_completion_claim_and_expiry_fence",
            "multipart_mixed_version_sql_fences",
            "multipart_completion_retry_backoff_quarantine_and_queue_fairness",
            "multipart_completion_post_expand_n_minus_one_scheduler_visibility",
            "multipart_completion_n_minus_one_terminalization_and_claim_promotion",
            "multipart_completion_preexisting_authority_backfill",
            "multipart_completion_lost_ack_clock_monotonicity",
            "multipart_completion_expired_final_attempt_terminalization",
            "multipart_complete_abort_linearization",
            "multipart_private_head_and_cross_part_range",
            "multipart_abort_idempotency_and_stale_cleanup",
            "multipart_cross_tenant_rejection",
            "exact_loopback_route_and_method_guard",
        ],
        "source_digest_sha256": digest_sources(),
        "result": "pass",
        "not_claimed": [
            "hosted_r2_behavior",
            "provider_network_or_quota_failures",
            "production_credentials_or_bucket_access",
            "automatic_recovery_after_provider_create_before_d1_handle_bind",
            "durability_latency_residency_lifecycle_or_cost",
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
        default=ROOT / "target" / "evidence" / "r2-storage-conformance.json",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        refuse_external_authority()
        verify_checked_in_surface()
        verify_mixed_version_sql_fences()
        verify_completion_reconciliation_sqlite()
        wrangler = detect_wrangler(arguments.wrangler_bin)
        with tempfile.TemporaryDirectory(prefix="frame-r2-conformance-") as directory:
            server = WorkerServer(wrangler, pathlib.Path(directory))
            try:
                server.start()
                payload = exercise_worker(server)
            finally:
                server.stop()
        write_evidence(arguments.evidence.resolve(), payload)
    except (ConformanceFailure, OSError, subprocess.SubprocessError, ValueError) as error:
        print(f"R2 storage conformance failed: {error}", file=sys.stderr)
        return 1
    print(
        "R2 Worker-binding conformance passed through compiled Worker "
        f"(credential-free local binding; Wrangler {WRANGLER_VERSION})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
